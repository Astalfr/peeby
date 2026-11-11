//! A grid of videos, loading the next page as the end comes into view: Home (recommendations),
//! subscriptions, the watch history, or a channel's "Videos" tab. Pages that need a session link
//! to the account page. The subscriptions page can also show its videos as a list (`FeedLayout`);
//! the history is always a list, as on youtube.com. A channel's videos sort by YouTube's chips
//! (latest, popular, oldest), and its search box shows the channel's videos matching a text
//! instead, until it is emptied.
//!
//! The videos are a virtual list (GPUI's `list`): each item is a row of cards (or one list row),
//! and only the items in view are built on a frame, so scrolling a long feed stays smooth.

use std::collections::HashSet;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
  AnyElement, Context, Entity, ListAlignment, ListOffset, ListState, Pixels, ScrollHandle,
  SharedString, Subscription, Window, div, list, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::notification::Notification;
use gpui_component::scroll::ScrollableElement;
use gpui_component::{IconName, Selectable, WindowExt, h_flex, v_flex};
use innertube::{Client, Error, Feed, FeedChip, PlaylistSummary, Related, VideoItem};

use crate::cards::{playlist_grid_card, playlist_result_row, video_card, video_row};
use crate::convert::error_text;
use crate::save::CardAction;
use crate::{Nav, Route};
use peeby_core::settings::{FeedLayout, Settings};
use ui::assets::Icon;
use ui::card::{skeleton_card, skeleton_row};
use ui::i18n::t;
use ui::images::Images;
use ui::widgets::{loading_more, notice, page_title};
use ui::{motion, runtime, tf, theme};

/// Narrowest card, logical pixels: a row holds as many cards as fit at this width, then they
/// widen to fill it.
const MIN_CARD_WIDTH: f32 = 416.;
/// Space between two cards of a row (`gap_x_4`).
const CARD_GAP: f32 = 16.;
/// Side margins of the page (`px_8`).
const PAGE_MARGIN: f32 = 32.;
/// Space above the first row, for a hovered card's rise (4 px), ring and shadow.
const TOP_ROOM: f32 = 8.;
/// Height of the notice in the place of a list row marked "Not interested".
const DISMISSED_ROW_HEIGHT: f32 = 96.;
/// Height built beyond the visible part of the list, above and below.
const OVERDRAW: Pixels = px(800.);

/// What the grid shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
  Home,
  Subscriptions,
  /// The account's watch history, most recent first.
  History,
  /// A channel's videos, newest first unless sorted otherwise.
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
  nav: Nav,
  state: State,
  /// The videos, and the playlists between them (mixes on Home).
  cards: Vec<Related>,
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
  /// A channel's sort orders, from its first page; kept while a search shows.
  chips: Vec<FeedChip>,
  /// Token of the sort order picked; `None`: the channel's default (newest first).
  sort: Option<String>,
  /// The text the channel's videos are searched for; `None`: its videos.
  query: Option<String>,
  /// A channel's search box.
  search: Option<Entity<InputState>>,
  /// Counts the first-page loads, so the answer of an older one (another sort, another search)
  /// is dropped.
  generation: u32,
  /// The videos marked "Not interested" (their menu's): each shows a notice in its place, with
  /// "Undo", until the page loads again.
  dismissed: HashSet<String>,
  _search_subscription: Option<Subscription>,
}

impl FeedView {
  pub fn new(
    client: Client,
    source: Source,
    nav: Nav,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let search = matches!(source, Source::Channel { .. }).then(|| {
      cx.new(|cx| {
        InputState::new(window, cx)
          .placeholder(t("Search the channel", "Rechercher dans la chaîne"))
      })
    });
    // Enter searches; emptying the box (by hand or with its clear button) goes back to the
    // videos.
    let search_subscription = search.as_ref().map(|input| {
      cx.subscribe(input, |this, input, event: &InputEvent, cx| {
        let text = input.read(cx).value().trim().to_string();
        match event {
          InputEvent::PressEnter { .. } => this.set_query((!text.is_empty()).then_some(text), cx),
          InputEvent::Change if text.is_empty() => this.set_query(None, cx),
          _ => {}
        }
      })
    });
    let mut view = Self {
      client,
      source,
      nav,
      state: State::Loading,
      cards: Vec::new(),
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
      chips: Vec::new(),
      sort: None,
      query: None,
      search,
      generation: 0,
      dismissed: HashSet::new(),
      _search_subscription: search_subscription,
    };
    view.load(None, cx);
    view
  }

  /// Loads the first page (`token` = `None`) or the next one.
  fn load(&mut self, token: Option<String>, cx: &mut Context<Self>) {
    let client = self.client.clone();
    let source = self.source.clone();
    let (sort, query) = (self.sort.clone(), self.query.clone());
    let more = token.is_some();
    if !more {
      self.generation += 1;
    }
    let generation = self.generation;
    self.loading_more = more;
    self.more_failed = false;
    let fetch = runtime::spawn(cx, async move {
      match (token, source) {
        (Some(token), Source::Subscriptions) => client.subscriptions_more(&token).await,
        (Some(token), Source::Channel { .. }) if query.is_some() => {
          client.channel_search_more(&token).await
        }
        (Some(token), _) => client.continuation(&token).await,
        (None, Source::Home) => client.home().await,
        (None, Source::Subscriptions) => client.subscriptions().await,
        (None, Source::History) => client.history().await,
        (None, Source::Channel { id, .. }) => match (query, sort) {
          (Some(query), _) => client.channel_search(&id, &query).await,
          (None, Some(sort)) => client.continuation(&sort).await,
          (None, None) => client.channel_videos(&id).await,
        },
      }
    });
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |this, cx| {
          if this.generation != generation {
            return;
          }
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

  /// Loads the first page again, for another sort order or search.
  fn reload(&mut self, cx: &mut Context<Self>) {
    self.state = State::Loading;
    self.continuation = None;
    self.load(None, cx);
    cx.notify();
  }

  /// Sorts a channel's videos by the chip whose token is `token`.
  fn set_sort(&mut self, token: String, cx: &mut Context<Self>) {
    if self.chips.iter().any(|c| c.selected && c.token == token) {
      return;
    }
    for chip in &mut self.chips {
      chip.selected = chip.token == token;
    }
    self.sort = Some(token);
    self.reload(cx);
  }

  /// Searches a channel's videos for `query`, or shows them all again (`None`).
  fn set_query(&mut self, query: Option<String>, cx: &mut Context<Self>) {
    if self.query == query {
      return;
    }
    self.query = query;
    self.reload(cx);
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
      self.cards.clear();
      self.dismissed.clear();
      self.logged_in = feed.logged_in;
      self.message = feed.message;
      // A search's results have no chips: the sort orders stay for when it ends.
      if !feed.chips.is_empty() {
        self.chips = feed.chips;
      }
    }
    // YouTube repeats some videos (twice on a page, or again in the next one). A card's id is
    // its video (or playlist) id, and two elements with one id share GPUI's hover state, which
    // then flips on every frame: keep the first.
    let received = feed.items.len() + feed.playlists.len();
    let before = self.cards.len();
    let old_count = self.shape.map(|shape| self.item_count(shape));
    for card in merge(feed.items, feed.playlists) {
      if !self.cards.iter().any(|c| key(c) == key(&card)) {
        self.cards.push(card);
      }
    }
    let repeated = received - (self.cards.len() - before);
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
    self.cards.len().div_ceil(shape.per_item()) + 1
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
    if start >= self.cards.len() {
      self.load_more(cx);
      return div()
        .px_8()
        .pb_6()
        .opacity(self.fade)
        .children(self.end_of_list(cx))
        .into_any_element();
    }
    let cards = &self.cards[start..(start + shape.per_item()).min(self.cards.len())];
    let row = if shape.list {
      div().pb_2().child(match &cards[0] {
        Related::Video(video) if self.dismissed.contains(&video.id) => {
          self.dismissed_notice(video, None, cx)
        }
        Related::Video(video) => {
          let not_interested = self.not_interested_action(video, cx);
          video_row(video, &self.nav, not_interested, window, cx)
        }
        Related::Playlist(playlist) => playlist_result_row(playlist, &self.nav, window, cx),
      })
    } else {
      // `h_flex` centers its children vertically: cards whose title takes one line would sit
      // lower than their neighbors.
      h_flex()
        .pb_8()
        .items_start()
        .gap_x_4()
        .children(cards.iter().map(|card| match card {
          Related::Video(video) if self.dismissed.contains(&video.id) => {
            self.dismissed_notice(video, Some(self.card_width), cx)
          }
          Related::Video(video) => {
            let not_interested = self.not_interested_action(video, cx);
            video_card(
              video,
              self.card_width,
              &self.nav,
              not_interested,
              window,
              cx,
            )
          }
          Related::Playlist(playlist) => {
            playlist_grid_card(playlist, self.card_width, &self.nav, window, cx)
          }
        }))
    };
    // Room above the first row: a hovered card rises over its box, and the list clips it.
    div()
      .px_8()
      .when(ix == 0, |el| el.pt(px(TOP_ROOM)))
      .opacity(self.fade)
      .child(row)
      .into_any_element()
  }

  /// The "Not interested" of `video`'s menu, when YouTube offers it (a recommendation, signed
  /// in).
  fn not_interested_action(&self, video: &VideoItem, cx: &mut Context<Self>) -> Option<CardAction> {
    video.not_interested.as_ref()?;
    let (view, id) = (cx.weak_entity(), video.id.clone());
    Some(Rc::new(move |window: &mut Window, cx: &mut gpui::App| {
      view
        .update(cx, |this, cx| {
          this.set_not_interested(&id, true, window, cx)
        })
        .ok();
    }))
  }

  /// Marks video `id` "Not interested" (`dismiss`), or undoes it, as youtube.com's menu and
  /// notice do: its card turns into the notice (or back) at once, and back again if YouTube
  /// refuses, with an error toast.
  fn set_not_interested(
    &mut self,
    id: &str,
    dismiss: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let tokens = self.cards.iter().find_map(|card| match card {
      Related::Video(video) if video.id == id => video.not_interested.clone(),
      _ => None,
    });
    let Some(tokens) = tokens else { return };
    let token = if dismiss {
      Some(tokens.token)
    } else {
      tokens.undo
    };
    let Some(token) = token else { return };
    let changed = if dismiss {
      self.dismissed.insert(id.to_owned())
    } else {
      self.dismissed.remove(id)
    };
    if !changed {
      return;
    }
    cx.notify();
    let client = self.client.clone();
    let task = runtime::spawn(cx, async move { client.send_feedback(&token).await });
    let (handle, id) = (window.window_handle(), id.to_owned());
    cx.spawn(async move |this, cx| {
      let Some(Err(e)) = task.await else { return };
      tracing::warn!("not interested: {e}");
      let note = Notification::error(if dismiss {
        tf!(
          "Cannot hide the video. {}",
          "Impossible d'ignorer la vidéo. {}",
          error_text(&e)
        )
      } else {
        tf!(
          "Cannot bring the video back. {}",
          "Impossible de rétablir la vidéo. {}",
          error_text(&e)
        )
      });
      cx.update(|cx| {
        this
          .update(cx, |this, cx| {
            if dismiss {
              this.dismissed.remove(&id);
            } else {
              this.dismissed.insert(id.clone());
            }
            cx.notify();
          })
          .ok();
        handle
          .update(cx, |_, window, cx| window.push_notification(note, cx))
          .ok();
      });
    })
    .detach();
  }

  /// In the place of a video marked "Not interested", as on youtube.com: "Video removed" and
  /// "Undo" (when YouTube gave its token), a grid card's size (`width`), or a list row's.
  fn dismissed_notice(
    &self,
    video: &VideoItem,
    width: Option<f32>,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let id = video.id.clone();
    let undo = video
      .not_interested
      .as_ref()
      .is_some_and(|n| n.undo.is_some());
    v_flex()
      .id(SharedString::from(format!("dismissed-{id}")))
      .flex_none()
      .map(|el| match width {
        Some(width) => el.w(px(width)).h(px(width * 9. / 16.)),
        None => el.w_full().h(px(DISMISSED_ROW_HEIGHT)),
      })
      .gap_2()
      .items_center()
      .justify_center()
      .rounded(theme::RADIUS)
      .bg(theme::surface())
      .text_sm()
      .text_color(theme::muted())
      .child(t("Video removed", "Vidéo supprimée"))
      .when(undo, |el| {
        el.child(
          Button::new(SharedString::from(format!("undo-{id}")))
            .ghost()
            .label(t("Undo", "Annuler"))
            .on_click(cx.listener(move |this, _, window, cx| {
              this.set_not_interested(&id, false, window, cx)
            })),
        )
      })
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
      Source::History => t("History", "Historique").into(),
      Source::Channel { name, .. } => name.clone(),
    }
  }

  fn empty_state(&self) -> impl IntoElement + use<> {
    let (title, body) = match (&self.state, &self.source, self.logged_in) {
      (State::Expired, _, _) => (
        t("Session expired", "Session expirée").to_owned(),
        t(
          "YouTube no longer recognizes the session. Sign in again.",
          "YouTube ne reconnaît plus la session. Reconnectez-vous.",
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
          "YouTube no longer shows recommendations without an account.",
          "YouTube n'affiche plus de recommandations sans compte.",
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
      (_, Source::History, _) => (
        self
          .message
          .clone()
          .unwrap_or_else(|| t("No history", "Aucun historique").into()),
        t(
          "The videos watched on this account appear here, unless the watch history is turned off.",
          "Les vidéos regardées sur ce compte apparaissent ici, sauf si l'historique des vidéos regardées est désactivé.",
        )
        .to_owned(),
      ),
      (_, Source::Channel { .. }, _) if self.query.is_some() => (
        t("No results", "Aucun résultat").to_owned(),
        t(
          "No video of this channel matches this search.",
          "Aucune vidéo de cette chaîne ne correspond à cette recherche.",
        )
        .to_owned(),
      ),
      _ => (t("No videos", "Aucune vidéo").to_owned(), String::new()),
    };
    let needs_session = matches!(self.state, State::SignedOut | State::Expired)
      || (matches!(self.source, Source::Home) && !self.logged_in);
    let nav = self.nav.clone();
    notice(
      match (needs_session, &self.source) {
        (true, _) => Icon::Account,
        (false, Source::History) => Icon::History,
        (false, Source::Channel { .. }) if self.query.is_some() => Icon::Search,
        (false, _) => Icon::Home,
      },
      title,
      body,
    )
    .when(needs_session, |el| {
      el.child(
        h_flex().pt_2().child(
          Button::new("to-account")
            .primary()
            .label(t("Sign in", "Se connecter"))
            .on_click(move |_, _, cx| {
              nav.update(cx, |app, cx| app.sign_in(cx)).ok();
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

/// A channel's sort orders, as YouTube names them; hidden while a search shows (its results
/// have their own order).
fn sort_switch(chips: &[FeedChip], cx: &mut Context<FeedView>) -> impl IntoElement {
  h_flex()
    .p_0p5()
    .gap_0p5()
    .rounded(theme::RADIUS)
    .border_1()
    .border_color(theme::border())
    .children(chips.iter().enumerate().map(|(ix, chip)| {
      let token = chip.token.clone();
      Button::new(("sort", ix))
        .ghost()
        .label(chip.label.clone())
        .selected(chip.selected)
        .on_click(cx.listener(move |this, _, _, cx| this.set_sort(token.clone(), cx)))
    }))
}

/// Cards per row of the grid and their width, from the window: the content panel is the window
/// less the sidebar and the page margins, and the cards share it.
/// A page's videos with its playlists put back in their places ([`Feed::playlists`]).
fn merge(videos: Vec<VideoItem>, playlists: Vec<(usize, PlaylistSummary)>) -> Vec<Related> {
  let mut playlists = playlists.into_iter().peekable();
  let mut cards = Vec::new();
  for (i, video) in videos.into_iter().enumerate() {
    while let Some((_, playlist)) = playlists.next_if(|(before, _)| *before <= i) {
      cards.push(Related::Playlist(playlist));
    }
    cards.push(Related::Video(video));
  }
  cards.extend(playlists.map(|(_, playlist)| Related::Playlist(playlist)));
  cards
}

/// What tells two cards apart: the video or playlist id.
fn key(card: &Related) -> &str {
  match card {
    Related::Video(video) => &video.id,
    Related::Playlist(playlist) => &playlist.id,
  }
}

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
    let list_mode = match self.source {
      Source::Subscriptions => Settings::get(cx).subscriptions_layout == FeedLayout::List,
      Source::History => true,
      _ => false,
    };
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
    let body = if matches!(self.state, State::Loaded) && !self.cards.is_empty() {
      self.set_shape(Shape {
        list: list_mode,
        columns: if list_mode { 1 } else { columns },
      });
      div()
        .size_full()
        .relative()
        .child(
          list(
            self.list.clone(),
            cx.processor(|this, ix, window, cx| this.item(ix, window, cx)),
          )
          .size_full(),
        )
        // After the content: children paint in order, and a hovered row would cover it.
        .vertical_scrollbar(&self.list)
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
              .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
          ),
        )
        .into_any_element(),
        _ => self.empty_state().into_any_element(),
      };
      div()
        .size_full()
        .relative()
        .child(
          div()
            .id("feed")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(div().px_8().pt(px(TOP_ROOM)).pb_6().child(content)),
        )
        .vertical_scrollbar(&self.scroll)
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
              .on_click(move |_, window, cx| nav.open(Route::Channels, window, cx)),
          )
        })
        .when(
          matches!(self.source, Source::Channel { .. })
            && self.query.is_none()
            && !self.chips.is_empty(),
          |el| el.child(sort_switch(&self.chips, cx)),
        )
        .children(self.search.as_ref().map(|input| {
          div().w(px(280.)).child(
            Input::new(input).cleanable(true).prefix(
              gpui_component::Icon::new(Icon::Search)
                .size(px(16.))
                .text_color(theme::muted()),
            ),
          )
        }))
    });
    v_flex()
      .size_full()
      .pt_6()
      .children(header)
      .child(div().flex_1().min_h_0().child(body))
  }
}

#[cfg(test)]
mod tests {
  use super::{key, merge};
  use innertube::{PlaylistSummary, VideoItem};

  fn video(id: &str) -> VideoItem {
    VideoItem {
      id: id.into(),
      title: String::new(),
      channel: None,
      channel_id: None,
      metadata: Vec::new(),
      duration: None,
      live: false,
      members_only: false,
      thumbnails: Vec::new(),
      avatar: Vec::new(),
      set_video_id: None,
      progress: None,
      not_interested: None,
    }
  }

  fn playlist(id: &str) -> PlaylistSummary {
    PlaylistSummary {
      id: id.into(),
      title: String::new(),
      count: None,
      channel: None,
      channel_id: None,
      metadata: Vec::new(),
      thumbnails: Vec::new(),
    }
  }

  #[test]
  fn playlists_go_back_in_their_places() {
    let videos = vec![video("a"), video("b"), video("c")];
    let playlists = vec![
      (0, playlist("RD0")),
      (2, playlist("RD2")),
      (3, playlist("RD3")),
    ];
    let cards = merge(videos, playlists);
    let keys: Vec<&str> = cards.iter().map(key).collect();
    assert_eq!(keys, ["RD0", "a", "b", "RD2", "c", "RD3"]);
  }
}
