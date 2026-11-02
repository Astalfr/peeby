//! Search across movies, series and episodes, as the user types (opened from the top bar).

use std::time::Duration;

use gpui::prelude::*;
use gpui::{AppContext, Context, Entity, Subscription, Task, WeakEntity, Window, div, px};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::scroll::ScrollableElement;
use gpui_component::v_flex;
use jellyfin_api::models::{BaseItem, ItemKind};
use jellyfin_api::{Client, ItemsQuery};
use ui::i18n::t;
use ui::images::Images;
use ui::widgets::page_title;
use ui::{runtime, theme};

use super::items::{Shape, card, route_for, section_title};
use super::{JellyfinSpace, report};

/// Typing pauses this long before the search runs.
const DEBOUNCE: Duration = Duration::from_millis(300);
const LIMIT: u32 = 60;

pub struct SearchView {
  client: Client,
  nav: WeakEntity<JellyfinSpace>,
  input: Entity<InputState>,
  results: Vec<BaseItem>,
  searched: Option<String>,
  pending: Option<Task<()>>,
  _subscription: Subscription,
}

impl SearchView {
  pub fn new(
    client: Client,
    nav: WeakEntity<JellyfinSpace>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let input = cx.new(|cx| {
      InputState::new(window, cx)
        .placeholder(t("Movies, shows, episodes…", "Films, séries, épisodes…"))
    });
    input.update(cx, |state, cx| state.focus(window, cx));
    let subscription = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| {
      if matches!(event, InputEvent::Change) {
        this.schedule(cx);
      }
    });
    Self {
      client,
      nav,
      input,
      results: Vec::new(),
      searched: None,
      pending: None,
      _subscription: subscription,
    }
  }

  /// Runs the shown search again, e.g. after playback; the results stay until the new ones
  /// arrive.
  pub fn reload(&mut self, cx: &mut Context<Self>) {
    if self.searched.is_some() {
      self.schedule(cx);
    }
  }

  /// Runs the search once typing pauses; a new keystroke replaces (cancels) the pending one.
  fn schedule(&mut self, cx: &mut Context<Self>) {
    let term = self.input.read(cx).value().trim().to_string();
    if term.is_empty() {
      self.pending = None;
      self.results.clear();
      self.searched = None;
      cx.notify();
      return;
    }
    let client = self.client.clone();
    self.pending = Some(cx.spawn(async move |this, cx| {
      cx.background_executor().timer(DEBOUNCE).await;
      let query = ItemsQuery {
        search_term: Some(term.clone()),
        include_item_types: vec![ItemKind::Movie, ItemKind::Series, ItemKind::Episode],
        recursive: true,
        limit: Some(LIMIT),
        ..Default::default()
      };
      let Ok(fetch) = this.update(cx, |_, cx| {
        runtime::spawn(cx, async move { client.items(&query).await })
      }) else {
        return;
      };
      let result = fetch.await;
      this
        .update(cx, |this, cx| {
          match result {
            Some(Ok(page)) => {
              this.results = page.items;
              this.searched = Some(term);
            }
            Some(Err(e)) => report(&this.nav, &e, cx),
            None => {}
          }
          cx.notify();
        })
        .ok();
    }));
  }
}

impl Render for SearchView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let (titles, episodes): (Vec<_>, Vec<_>) = self
      .results
      .iter()
      .partition(|i| i.kind != ItemKind::Episode);
    let mut cards = |scope: &str, items: &[&BaseItem], shape: Shape, window: &mut Window| {
      items
        .iter()
        .map(|item| {
          card(
            scope,
            item,
            shape,
            route_for(item),
            &self.nav,
            &self.client,
            window,
            cx,
          )
        })
        .collect::<Vec<_>>()
    };
    let title_cards = cards("titles", &titles, Shape::Poster, window);
    let episode_cards = cards("episodes", &episodes, Shape::Thumb, window);
    let empty = self.searched.is_some() && self.results.is_empty();

    v_flex()
      .id("jellyfin-search")
      .size_full()
      .overflow_y_scrollbar()
      .p_8()
      .gap_6()
      .child(page_title(t("Search", "Recherche")))
      .child(div().max_w(px(560.)).child(Input::new(&self.input)))
      .when(empty, |el| {
        el.child(
          div()
            .text_color(theme::muted())
            .child(t("No results.", "Aucun résultat.")),
        )
      })
      .when(!title_cards.is_empty(), |el| {
        el.child(div().flex().flex_wrap().gap_4().children(title_cards))
      })
      .when(!episode_cards.is_empty(), |el| {
        el.child(section_title(t("Episodes", "Épisodes")))
          .child(div().flex().flex_wrap().gap_4().children(episode_cards))
      })
  }
}
