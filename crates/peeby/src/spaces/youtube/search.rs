//! Search (opened from the top bar): a query run on Enter, then the videos and channels found,
//! as list rows, with the next pages loaded while scrolling.

use gpui::prelude::*;
use gpui::{
  AppContext, Context, Entity, FontWeight, ScrollHandle, SharedString, Subscription, WeakEntity,
  Window, div, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, v_flex};
use innertube::{Channel, Client, Error, SearchResult};

use crate::spaces::youtube::cards::video_row;
use crate::spaces::youtube::convert::{self, error_text};
use crate::spaces::youtube::{Route, YouTubeSpace};
use ui::assets::Icon;
use ui::card::{ROW_WIDTH, WIDE, skeleton_row};
use ui::i18n::t;
use ui::images::Images;
use ui::picture::avatar;
use ui::widgets::{loading_more, near_scroll_end, notice, page_title};
use ui::{runtime, theme};

/// Side of a channel's avatar in the results.
const CHANNEL_AVATAR: f32 = 136.;

enum State {
  /// Nothing searched yet.
  Idle,
  Loading,
  Loaded {
    results: Vec<SearchResult>,
    continuation: Option<String>,
  },
  Failed(SharedString),
}

pub struct SearchView {
  client: Client,
  nav: WeakEntity<YouTubeSpace>,
  input: Entity<InputState>,
  state: State,
  loading_more: bool,
  /// The last next-page request failed: no automatic retry, the button retries.
  more_failed: bool,
  /// Counts the searches, so the answer of an older one is dropped.
  generation: u32,
  scroll: ScrollHandle,
  _input_subscription: Subscription,
}

impl SearchView {
  pub fn new(
    client: Client,
    nav: WeakEntity<YouTubeSpace>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let input = cx.new(|cx| {
      InputState::new(window, cx).placeholder(t("Search YouTube", "Rechercher sur YouTube"))
    });
    input.update(cx, |state, cx| state.focus(window, cx));
    let subscription = cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
      if matches!(event, InputEvent::PressEnter { .. }) {
        this.search(cx);
      }
    });
    Self {
      client,
      nav,
      input,
      state: State::Idle,
      loading_more: false,
      more_failed: false,
      generation: 0,
      scroll: ScrollHandle::new(),
      _input_subscription: subscription,
    }
  }

  /// Runs the typed query (Enter).
  fn search(&mut self, cx: &mut Context<Self>) {
    let query = self.input.read(cx).value().trim().to_owned();
    if query.is_empty() {
      return;
    }
    self.generation += 1;
    let generation = self.generation;
    self.state = State::Loading;
    self.loading_more = false;
    self.more_failed = false;
    self.scroll.set_offset(gpui::point(px(0.), px(0.)));
    cx.notify();
    let client = self.client.clone();
    let fetch = runtime::spawn(cx, async move { client.search(&query).await });
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |this, cx| {
          if this.generation != generation {
            return;
          }
          this.state = match result {
            Some(Ok(page)) => State::Loaded {
              results: page.results,
              continuation: page.continuation,
            },
            Some(Err(e)) => State::Failed(failure(&e)),
            None => return,
          };
          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  /// Loads the next page once the end of the results comes within about a screen.
  fn load_more_if_near_end(&mut self, cx: &mut Context<Self>) {
    let State::Loaded { continuation, .. } = &self.state else {
      return;
    };
    if self.loading_more || self.more_failed || !near_scroll_end(&self.scroll) {
      return;
    }
    let Some(token) = continuation.clone() else {
      return;
    };
    self.load_more(token, cx);
  }

  fn load_more(&mut self, token: String, cx: &mut Context<Self>) {
    self.loading_more = true;
    self.more_failed = false;
    let generation = self.generation;
    let client = self.client.clone();
    let fetch = runtime::spawn(cx, async move { client.search_more(&token).await });
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |this, cx| {
          if this.generation != generation {
            return;
          }
          this.loading_more = false;
          let State::Loaded {
            results,
            continuation,
          } = &mut this.state
          else {
            return;
          };
          match result {
            Some(Ok(page)) => {
              // A video can come back on a later page: keep its first place.
              for result in page.results {
                if !results.iter().any(|r| key(r) == key(&result)) {
                  results.push(result);
                }
              }
              *continuation = page.continuation;
            }
            Some(Err(e)) => {
              tracing::warn!("search, next page: {e}");
              this.more_failed = true;
            }
            None => {}
          }
          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  /// A channel found: its avatar in the place of a thumbnail, its name, handle and subscribers.
  fn channel_row(
    &self,
    channel: &Channel,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let route = Route::Channel {
      id: channel.id.clone(),
      name: channel.name.clone(),
    };
    let nav = self.nav.clone();
    let group = SharedString::from(format!("search-channel-{}", channel.id));
    let details = [channel.handle.clone(), channel.subscribers.clone()]
      .into_iter()
      .flatten()
      .collect::<Vec<_>>()
      .join(" · ");
    h_flex()
      .id(group.clone())
      .group(group.clone())
      .w_full()
      .p_2()
      .gap_4()
      .items_center()
      .rounded(theme::RADIUS_LG)
      .cursor_pointer()
      .hover(|s| s.bg(theme::surface()))
      .on_click(move |_, window, cx| {
        nav
          .update(cx, |app, cx| app.open(route.clone(), window, cx))
          .ok();
      })
      .child(
        div()
          .w(px(ROW_WIDTH))
          .h(px(ROW_WIDTH / WIDE))
          .flex_none()
          .flex()
          .items_center()
          .justify_center()
          .child(avatar(
            &convert::picture(&channel.avatar),
            CHANNEL_AVATAR,
            window,
            cx,
          )),
      )
      .child(
        v_flex()
          .flex_1()
          .min_w_0()
          .gap_1()
          .child(
            div()
              .text_lg()
              .font_weight(FontWeight::SEMIBOLD)
              .truncate()
              .group_hover(group, |s| s.text_color(theme::accent_hover()))
              .child(channel.name.clone()),
          )
          .child(
            div()
              .text_sm()
              .text_color(theme::muted())
              .truncate()
              .child(details),
          ),
      )
      .into_any_element()
  }
}

/// What tells two results apart: the video or channel id.
fn key(result: &SearchResult) -> &str {
  match result {
    SearchResult::Video(video) => &video.id,
    SearchResult::Channel(channel) => &channel.id,
  }
}

/// What the user reads when a search fails.
fn failure(error: &Error) -> SharedString {
  tracing::warn!("search: {error}");
  match error {
    Error::NotLoggedIn => t(
      "YouTube no longer accepts the session: import new cookies from Account.",
      "YouTube n'accepte plus la session : importez de nouveaux cookies depuis Compte.",
    )
    .into(),
    e => error_text(e).into(),
  }
}

impl Render for SearchView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    self.load_more_if_near_end(cx);
    let body = match &self.state {
      State::Idle => div().into_any_element(),
      State::Loading => v_flex()
        .gap_2()
        .children((0..4).map(|_| skeleton_row()))
        .into_any_element(),
      State::Failed(message) => notice(
        Icon::Search,
        t("Search failed", "Recherche impossible"),
        message.clone(),
      )
      .into_any_element(),
      State::Loaded { results, .. } if results.is_empty() => notice(
        Icon::Search,
        t("No results", "Aucun résultat"),
        t("Try other words.", "Essayez d'autres mots."),
      )
      .into_any_element(),
      State::Loaded {
        results,
        continuation,
      } => {
        let rows: Vec<_> = results
          .clone()
          .iter()
          .map(|result| match result {
            SearchResult::Video(video) => video_row(video, &self.nav, window, cx),
            SearchResult::Channel(channel) => self.channel_row(channel, window, cx),
          })
          .collect();
        let end = if self.more_failed {
          continuation.clone().map(|token| {
            Button::new("search-retry")
              .ghost()
              .label(t("Retry", "Réessayer"))
              .on_click(cx.listener(move |this, _, _, cx| this.load_more(token.clone(), cx)))
              .into_any_element()
          })
        } else {
          self.loading_more.then(|| loading_more().into_any_element())
        };
        v_flex()
          .gap_1()
          .children(rows)
          .children(end)
          .into_any_element()
      }
    };
    div()
      .size_full()
      .relative()
      .vertical_scrollbar(&self.scroll)
      .child(
        v_flex()
          .id("youtube-search")
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
              .max_w(px(1200.))
              .child(page_title(t("Search", "Recherche")))
              .child(div().max_w(px(640.)).child(Input::new(&self.input)))
              .child(body),
          ),
      )
  }
}
