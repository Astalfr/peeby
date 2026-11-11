//! Search (opened from the top bar): a query run on Enter, then the videos and channels found,
//! as list rows, with the next pages loaded while scrolling.
//!
//! While typing, YouTube's suggestions show under the field, as on youtube.com
//! (`Client::search_suggestions`, asked once the typing pauses): the arrows move through them,
//! Enter or a click runs the one picked (a channel's opens its page), Escape closes them.

use gpui::prelude::*;
use std::time::Duration;

use gpui::{
  AppContext, Context, Entity, Focusable, FontWeight, MouseButton, ScrollHandle, SharedString,
  Subscription, Task, Window, deferred, div, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, v_flex};
use innertube::{Channel, Client, Error, SearchResult, SearchSuggestion, Thumbnail};

use crate::cards::{playlist_result_row, video_row};
use crate::convert::{self, error_text};
use crate::{Nav, Route};
use peeby_core::shell::space::middle_click;
use ui::assets::Icon;
use ui::card::{ROW_WIDTH, WIDE, skeleton_row};
use ui::i18n::t;
use ui::images::Images;
use ui::picture::avatar;
use ui::widgets::{loading_more, near_scroll_end, notice, page_title};
use ui::{runtime, theme};

/// Side of a channel's avatar in the results.
const CHANNEL_AVATAR: f32 = 136.;
/// How long the typing pauses before the suggestions are asked for.
const SUGGEST_DELAY: Duration = Duration::from_millis(150);
/// Suggestions shown at most.
const MAX_SUGGESTIONS: usize = 10;
/// Width of the search field and of its suggestions.
const FIELD_WIDTH: f32 = 640.;

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
  nav: Nav,
  input: Entity<InputState>,
  state: State,
  loading_more: bool,
  /// The last next-page request failed: no automatic retry, the button retries.
  more_failed: bool,
  /// Counts the searches, so the answer of an older one is dropped.
  generation: u32,
  scroll: ScrollHandle,
  /// YouTube's suggestions for what is typed.
  suggestions: Vec<SearchSuggestion>,
  /// The suggestion the arrows are on.
  highlighted: Option<usize>,
  /// The suggestions show under the field.
  suggestions_open: bool,
  /// Counts the edits, so the suggestions of an older text are dropped.
  suggest_generation: u32,
  /// The next change of the field is the program's (a suggestion picked), not typing.
  ignore_change: bool,
  _suggest_task: Option<Task<()>>,
  _close_task: Option<Task<()>>,
  _input_subscription: Subscription,
  _keys: Subscription,
}

impl SearchView {
  pub fn new(client: Client, nav: Nav, window: &mut Window, cx: &mut Context<Self>) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let input = cx.new(|cx| {
      InputState::new(window, cx).placeholder(t("Search YouTube", "Rechercher sur YouTube"))
    });
    input.update(cx, |state, cx| state.focus(window, cx));
    let subscription =
      cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
        match event {
          InputEvent::PressEnter { .. } => match this.highlighted_suggestion() {
            Some(suggestion) => this.pick(suggestion, window, cx),
            None => {
              this.close_suggestions(cx);
              this.search(cx);
            }
          },
          InputEvent::Change if this.ignore_change => this.ignore_change = false,
          InputEvent::Change => this.suggest(cx),
          InputEvent::Focus if !this.suggestions.is_empty() => {
            this.suggestions_open = true;
            cx.notify();
          }
          // Later: a click on a suggestion takes the focus before it lands.
          InputEvent::Blur => {
            this._close_task = Some(cx.spawn(async move |this, cx| {
              cx.background_executor().timer(SUGGEST_DELAY).await;
              this.update(cx, |this, cx| this.close_suggestions(cx)).ok();
            }));
          }
          InputEvent::Focus => {}
        }
      });
    // The arrows and Escape move through the suggestions before the field (which binds them)
    // sees them, while it has the focus and they show.
    let view = cx.weak_entity();
    let keys = cx.intercept_keystrokes(move |event, window, cx| {
      let key = event.keystroke.key.as_str();
      if !matches!(key, "up" | "down" | "escape") || event.keystroke.modifiers.modified() {
        return;
      }
      view
        .update(cx, |this, cx| {
          let focused = this.input.read(cx).focus_handle(cx).is_focused(window);
          if !focused || !this.suggestions_open || this.suggestions.is_empty() {
            return;
          }
          let count = this.suggestions.len();
          this.highlighted = match (key, this.highlighted) {
            ("down", None) => Some(0),
            ("down", Some(i)) => Some((i + 1) % count),
            ("up", None | Some(0)) => Some(count - 1),
            ("up", Some(i)) => Some(i - 1),
            _ => {
              this.suggestions_open = false;
              None
            }
          };
          cx.stop_propagation();
          cx.notify();
        })
        .ok();
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
      suggestions: Vec::new(),
      highlighted: None,
      suggestions_open: false,
      suggest_generation: 0,
      ignore_change: false,
      _suggest_task: None,
      _close_task: None,
      _input_subscription: subscription,
      _keys: keys,
    }
  }

  /// The field changed: its suggestions once the typing pauses (none for an empty field).
  fn suggest(&mut self, cx: &mut Context<Self>) {
    self.suggest_generation += 1;
    let generation = self.suggest_generation;
    let text = self.input.read(cx).value().trim().to_owned();
    if text.is_empty() {
      self._suggest_task = None;
      self.suggestions.clear();
      self.close_suggestions(cx);
      return;
    }
    self._suggest_task = Some(cx.spawn(async move |this, cx| {
      cx.background_executor().timer(SUGGEST_DELAY).await;
      let fetch = this
        .update(cx, |this, cx| {
          let client = this.client.clone();
          (this.suggest_generation == generation)
            .then(|| runtime::spawn(cx, async move { client.search_suggestions(&text).await }))
        })
        .ok()
        .flatten();
      let Some(fetch) = fetch else { return };
      let result = fetch.await;
      this
        .update(cx, |this, cx| {
          if this.suggest_generation != generation {
            return;
          }
          match result {
            Some(Ok(suggestions)) => {
              this.suggestions = suggestions.into_iter().take(MAX_SUGGESTIONS).collect();
              this.highlighted = None;
              this.suggestions_open = !this.suggestions.is_empty();
              cx.notify();
            }
            // The typed query never reaches the log.
            Some(Err(e)) => tracing::debug!("search suggestions: {e}"),
            None => {}
          }
        })
        .ok();
    }));
  }

  fn highlighted_suggestion(&self) -> Option<SearchSuggestion> {
    let i = self.highlighted.filter(|_| self.suggestions_open)?;
    self.suggestions.get(i).cloned()
  }

  fn close_suggestions(&mut self, cx: &mut Context<Self>) {
    if self.suggestions_open || self.highlighted.is_some() {
      self.suggestions_open = false;
      self.highlighted = None;
      cx.notify();
    }
  }

  /// Runs `suggestion`: a channel's opens its page, any other is searched for, written in the
  /// field.
  fn pick(&mut self, suggestion: SearchSuggestion, window: &mut Window, cx: &mut Context<Self>) {
    self.close_suggestions(cx);
    if let Some(channel) = suggestion.channel {
      self.nav.open(
        Route::Channel {
          id: channel.id,
          name: channel.name,
        },
        window,
        cx,
      );
      return;
    }
    self.ignore_change = true;
    self.suggest_generation += 1;
    self
      .input
      .update(cx, |input, cx| input.set_value(suggestion.text, window, cx));
    self.search(cx);
  }

  /// The suggestions under the field, over the page: a search icon (a channel's avatar) and
  /// the text (the channel's name and handle), the one the arrows are on highlighted.
  fn suggestion_list(
    &self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Option<impl IntoElement> {
    if !self.suggestions_open || self.suggestions.is_empty() {
      return None;
    }
    let rows: Vec<_> = self
      .suggestions
      .iter()
      .enumerate()
      .map(|(i, suggestion)| {
        let icon = match &suggestion.channel {
          Some(channel) => {
            let picture = convert::picture(&[Thumbnail {
              url: channel.avatar.clone().unwrap_or_default(),
              width: 88,
              height: 88,
            }]);
            avatar(&picture, 24., window, cx)
          }
          None => div()
            .size(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .text_color(theme::muted())
            .child(gpui_component::Icon::new(Icon::Search).size(px(16.)))
            .into_any_element(),
        };
        let text = match &suggestion.channel {
          Some(channel) => h_flex()
            .gap_2()
            .min_w_0()
            .child(
              div()
                .font_weight(FontWeight::SEMIBOLD)
                .child(channel.name.clone()),
            )
            .children(
              channel
                .handle
                .clone()
                .map(|h| div().text_color(theme::muted()).child(h)),
            )
            .into_any_element(),
          None => div()
            .min_w_0()
            .truncate()
            .child(suggestion.text.clone())
            .into_any_element(),
        };
        let picked = suggestion.clone();
        h_flex()
          .id(("suggestion", i))
          .px_3()
          .py_1p5()
          .gap_3()
          .items_center()
          .rounded(theme::RADIUS)
          .cursor_pointer()
          .when(self.highlighted == Some(i), |el| {
            el.bg(theme::surface_hover())
          })
          .hover(|s| s.bg(theme::surface_hover()))
          .child(icon)
          .child(text)
          .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
              cx.stop_propagation();
              this.pick(picked.clone(), window, cx);
            }),
          )
      })
      .collect();
    Some(deferred(
      v_flex()
        .id("search-suggestions")
        .occlude()
        .absolute()
        .top_full()
        .left_0()
        .right_0()
        .mt_1()
        .p_1()
        .rounded(theme::RADIUS_LG)
        .border_1()
        .border_color(theme::border())
        .bg(theme::background())
        .shadow_lg()
        .text_sm()
        .children(rows),
    ))
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
      .on_aux_click(middle_click({
        let (nav, route) = (nav.clone(), route.clone());
        move |cx| nav.open_in_new_tab(route.clone(), None, cx)
      }))
      .on_click(move |_, window, cx| nav.open(route.clone(), window, cx))
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

/// What tells two results apart: the video, channel or playlist id.
fn key(result: &SearchResult) -> &str {
  match result {
    SearchResult::Video(video) => &video.id,
    SearchResult::Channel(channel) => &channel.id,
    SearchResult::Playlist(playlist) => &playlist.id,
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
    let suggestions = self
      .suggestion_list(window, cx)
      .map(IntoElement::into_any_element);
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
            SearchResult::Video(video) => video_row(video, &self.nav, None, window, cx),
            SearchResult::Channel(channel) => self.channel_row(channel, window, cx),
            SearchResult::Playlist(playlist) => {
              playlist_result_row(playlist, &self.nav, window, cx)
            }
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
              .child(
                div()
                  .relative()
                  .max_w(px(FIELD_WIDTH))
                  .child(Input::new(&self.input))
                  .children(suggestions),
              )
              .child(body),
          ),
      )
      // After the content: children paint in order, and a hovered row would cover it.
      .vertical_scrollbar(&self.scroll)
  }
}
