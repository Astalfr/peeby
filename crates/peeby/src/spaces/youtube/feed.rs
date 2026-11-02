//! A grid of videos, loading the next page as the end comes into view: Home (recommendations),
//! subscriptions, or a channel's "Videos" tab. Pages that need a session link to the account page.
//! The subscriptions page can also show its videos as a list (`FeedLayout`).
//!
//! The videos are a virtual list (GPUI's `list`): each item is a row of cards (or one list row),
//! and only the items in view are built on a frame, so scrolling a long feed stays smooth.

use gpui::prelude::*;
use gpui::{
  AnyElement, Context, ListAlignment, ListOffset, ListState, Pixels, ScrollHandle, SharedString,
  WeakEntity, Window, div, list, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{IconName, Selectable, h_flex, v_flex};
use innertube::{Client, Error, Feed, VideoItem};

use crate::settings::{FeedLayout, Settings};
use crate::spaces::youtube::cards::{video_card, video_row};
use crate::spaces::youtube::convert::error_text;
use crate::spaces::youtube::{Route, YouTubeSpace};
use ui::assets::Icon;
use ui::card::{skeleton_card, skeleton_row};
use ui::i18n::t;
use ui::images::Images;
use ui::widgets::{loading_more, notice, page_title};
use ui::{motion, runtime, theme};

/// Narrowest card, logical pixels: a row holds as many cards as fit at this width, then they
/// widen to fill it.
const MIN_CARD_WIDTH: f32 = 416.;
/// Space between two cards of a row (`gap_x_4`).
const CARD_GAP: f32 = 16.;
/// Side margins of the page (`px_8`).
const PAGE_MARGIN: f32 = 32.;
/// Space above the first row, for a hovered card's rise (4 px), ring and shadow.
const TOP_ROOM: f32 = 8.;
/// Height built beyond the visible part of the list, above and below.
const OVERDRAW: Pixels = px(800.);

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

/// What the list items are: list rows (`list`) or rows of `columns` cards.
#[derive(Clone, Copy, PartialEq)]
struct Shape {
  list: bool,
  columns: usize,
}

impl Shape {
  /// Videos per list item.
  fn per_item(self) -> usize {
    if self.list { 1 } else { self.columns }
  }
}

pub struct FeedView {
  client: Client,
  source: Source,
  nav: WeakEntity<YouTubeSpace>,
  state: State,
  videos: Vec<VideoItem>,
  continuation: Option<String>,
  loading_more: bool,
  /// The last next-page request failed: no automatic retry, the button retries.
  more_failed: bool,
  /// Scrolls the page while it shows a skeleton or a notice.
  scroll: ScrollHandle,
  /// The videos, as rows: one item per row, then the end of the list.
  list: ListState,
  /// The shape of the list's items; `None` until the next render rebuilds them.
  shape: Option<Shape>,
  /// Opacity of the videos as they fade in over the skeleton, set on each render.
  fade: f32,
  /// Width of a grid card, set on each render from the window.
  card_width: f32,
  logged_in: bool,
  message: Option<String>,
}

impl FeedView {
  pub fn new(
    client: Client,
    source: Source,
    nav: WeakEntity<YouTubeSpace>,
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
      list: ListState::new(0, ListAlignment::Top, OVERDRAW),
      shape: None,
      fade: 0.,
      card_width: MIN_CARD_WIDTH,
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
        (Some(token), Source::Subscriptions) => client.subscriptions_more(&token).await,
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

  /// Next page, unless one is loading or the last one failed (the button retries then).
  fn load_more(&mut self, cx: &mut Context<Self>) {
    if self.loading_more || self.more_failed {
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
    let old_count = self.shape.map(|shape| self.item_count(shape));
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
    match (self.shape, old_count) {
      // The row that held the last video may take more cards now, then come the new rows.
      (Some(shape), Some(old_count)) if more => {
        let first = before / shape.per_item();
        self
          .list
          .splice(first..old_count, self.item_count(shape) - first);
      }
      // A new first page: the next render rebuilds the list.
      _ => self.shape = None,
    }
  }

  /// List items for `shape`: the rows, then the end of the list.
  fn item_count(&self, shape: Shape) -> usize {
    self.videos.len().div_ceil(shape.per_item()) + 1
  }

  /// Rebuilds the list items when the layout or the number of cards per row changed, keeping
  /// the first video in view.
  fn set_shape(&mut self, shape: Shape) {
    if self.shape == Some(shape) {
      return;
    }
    let first = self.shape.map_or(0, |old| {
      self.list.logical_scroll_top().item_ix * old.per_item()
    });
    self.shape = Some(shape);
    self.list.reset(self.item_count(shape));
    self.list.scroll_to(ListOffset {
      item_ix: first / shape.per_item(),
      offset_in_item: px(0.),
    });
  }

  /// List item `ix`: a row of cards (or a list row), or the end of the list. The end coming
  /// within the list's reach (a screen and the overdraw) loads the next page.
  fn item(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    let Some(shape) = self.shape else {
      return div().into_any_element();
    };
    let start = ix * shape.per_item();
    if start >= self.videos.len() {
      self.load_more(cx);
      return div()
        .px_8()
        .pb_6()
        .opacity(self.fade)
        .children(self.end_of_list(cx))
        .into_any_element();
    }
    let videos = &self.videos[start..(start + shape.per_item()).min(self.videos.len())];
    let row = if shape.list {
      div()
        .pb_2()
        .child(video_row(&videos[0], &self.nav, window, cx))
    } else {
      // `h_flex` centers its children vertically: cards whose title takes one line would sit
      // lower than their neighbors.
      h_flex().pb_8().items_start().gap_x_4().children(
        videos
          .iter()
          .map(|v| video_card(v, self.card_width, &self.nav, window, cx)),
      )
    };
    // Room above the first row: a hovered card rises over its box, and the list clips it.
    div()
      .px_8()
      .when(ix == 0, |el| el.pt(px(TOP_ROOM)))
      .opacity(self.fade)
      .child(row)
      .into_any_element()
  }

  /// Under the last video: a spinner while the next page loads, the retry button after a
  /// failure (pages load by themselves otherwise).
  fn end_of_list(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
    let token = self.continuation.clone()?;
    let element = if self.more_failed {
      Button::new("more")
        .primary()
        .label(t("Retry", "Réessayer"))
        .on_click(cx.listener(move |this, _, _, cx| this.load(Some(token.clone()), cx)))
        .into_any_element()
    } else if self.loading_more {
      loading_more().into_any_element()
    } else {
      return None;
    };
    Some(h_flex().justify_center().child(element).into_any_element())
  }

  fn title(&self) -> String {
    match &self.source {
      Source::Home => t("Home", "Accueil").into(),
      Source::Subscriptions => t("Subscriptions", "Abonnements").into(),
      Source::Channel { name, .. } => name.clone(),
    }
  }

  fn empty_state(&self) -> impl IntoElement + use<> {
    let (title, body) = match (&self.state, &self.source, self.logged_in) {
      (State::Expired, _, _) => (
        t("Session expired", "Session expirée").to_owned(),
        t(
          "YouTube no longer recognizes the session's cookies. Import new ones from the \
           Account page.",
          "YouTube ne reconnaît plus les cookies de la session. Importez-en de nouveaux depuis \
           la page Compte.",
        )
        .to_owned(),
      ),
      (State::SignedOut, _, _) | (_, Source::Home, false) => (
        t(
          "Sign in to see this page",
          "Connectez-vous pour voir cette page",
        )
        .to_owned(),
        t(
          "YouTube no longer shows recommendations without an account. Import the cookies of \
           a YouTube session from the Account page.",
          "YouTube n'affiche plus de recommandations sans compte. Importez les cookies d'une \
           session YouTube depuis la page Compte.",
        )
        .to_owned(),
      ),
      (_, Source::Home, true) => (
        self
          .message
          .clone()
          .unwrap_or_else(|| t("No recommendations", "Aucune recommandation").into()),
        t(
          "YouTube has nothing to suggest for now: the watch history may be turned off on \
           this account.",
          "YouTube ne propose rien pour l'instant : l'historique des vidéos regardées est \
           peut-être désactivé sur ce compte.",
        )
        .to_owned(),
      ),
      _ => (t("No videos", "Aucune vidéo").to_owned(), String::new()),
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
            .label(t("Import cookies", "Importer des cookies"))
            .on_click(move |_, _, cx| {
              nav.update(cx, |app, cx| app.open_settings(cx)).ok();
            }),
        ),
      )
    })
  }
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
      t("Grid", "Grille"),
      FeedLayout::Grid,
    ))
    .child(choice(
      "layout-list",
      Icon::List,
      t("List", "Liste"),
      FeedLayout::List,
    ))
}

/// Cards per row of the grid and their width, from the window: the content panel is the window
/// less the sidebar and the page margins, and the cards share it.
fn grid(window: &Window, cx: &gpui::App) -> (usize, f32) {
  let sidebar = theme::sidebar_width(Settings::get(cx).sidebar_collapsed);
  let width = f32::from(window.viewport_size().width - sidebar) - 2. * PAGE_MARGIN;
  let columns = ((width + CARD_GAP) / (MIN_CARD_WIDTH + CARD_GAP))
    .floor()
    .max(1.);
  // Whole pixels: a fractional width would leave a hairline gap at the row's end.
  let card = ((width - CARD_GAP * (columns - 1.)) / columns)
    .floor()
    .max(MIN_CARD_WIDTH);
  (columns as usize, card)
}

impl Render for FeedView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let list_mode = matches!(self.source, Source::Subscriptions)
      && Settings::get(cx).subscriptions_layout == FeedLayout::List;
    let shown = if matches!(self.state, State::Loading) {
      0.
    } else {
      1.
    };
    // The content fades in while the skeleton, laid over it, fades out.
    self.fade = motion::tween(
      SharedString::from(format!("feed-load-{}", cx.entity_id())),
      shown,
      shown,
      theme::SLOW,
      window,
      cx,
    );
    let (columns, card_width) = grid(window, cx);
    self.card_width = card_width;
    let skeleton = if list_mode {
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
        .children((0..12).map(|_| skeleton_card(card_width)))
        .into_any_element()
    };
    let body = if matches!(self.state, State::Loaded) && !self.videos.is_empty() {
      self.set_shape(Shape {
        list: list_mode,
        columns: if list_mode { 1 } else { columns },
      });
      div()
        .size_full()
        .relative()
        .vertical_scrollbar(&self.list)
        .child(
          list(
            self.list.clone(),
            cx.processor(|this, ix, window, cx| this.item(ix, window, cx)),
          )
          .size_full(),
        )
        .when(self.fade < 1., |el| {
          el.child(
            div()
              .absolute()
              .top_0()
              .left_0()
              .right_0()
              .px_8()
              .pt(px(TOP_ROOM))
              .opacity(1. - self.fade)
              .child(skeleton),
          )
        })
        .into_any_element()
    } else {
      let content = match &self.state {
        State::Loading => skeleton,
        State::Failed(e) => notice(
          IconName::TriangleAlert,
          t("Cannot load the page", "Impossible de charger la page"),
          e.clone(),
        )
        .child(
          h_flex().pt_2().child(
            Button::new("retry")
              .primary()
              .label(t("Retry", "Réessayer"))
              .on_click(cx.listener(|this, _, _, cx| {
                this.state = State::Loading;
                this.load(None, cx);
                cx.notify();
              })),
          ),
        )
        .into_any_element(),
        _ => self.empty_state().into_any_element(),
      };
      div()
        .size_full()
        .relative()
        .vertical_scrollbar(&self.scroll)
        .child(
          div()
            .id("feed")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(div().px_8().pt(px(TOP_ROOM)).pb_6().child(content)),
        )
        .into_any_element()
    };
    // The header stays above the videos, which scroll under it.
    let header = (!matches!(self.source, Source::Home)).then(|| {
      h_flex()
        .px_8()
        .pb_6()
        .gap_4()
        .items_center()
        .child(page_title(self.title()))
        .child(div().flex_1())
        .when(matches!(self.source, Source::Subscriptions), |el| {
          let nav = self.nav.clone();
          el.child(layout_switch(list_mode, cx)).child(
            Button::new("all-channels")
              .icon(Icon::Subscriptions)
              .label(t("All channels", "Toutes les chaînes"))
              .on_click(move |_, window, cx| {
                nav
                  .update(cx, |app, cx| app.open(Route::Channels, window, cx))
                  .ok();
              }),
          )
        })
    });
    v_flex()
      .size_full()
      .pt_6()
      .children(header)
      .child(div().flex_1().min_h_0().child(body))
  }
}
