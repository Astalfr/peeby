//! "Parcourir": the categories with the most viewers (Helix `games/top`), and a category's page with
//! its live streams (`streams?game_id=`), most viewers first.

use gpui::prelude::*;
use gpui::{Context, FontWeight, Task, WeakEntity, Window, div};
use gpui_component::v_flex;
use twitch::helix::{Category, Helix, Stream};
use ui::i18n::t;
use ui::images::Images;
use ui::runtime;

use super::TwitchSpace;
use super::cards::{self, Avatars};

/// Pages of categories read (100 each).
const CATEGORY_PAGES: usize = 2;

/// Loads `fetch` on the network runtime and keeps its result in the view.
pub(super) fn load<V: 'static, T: Send + 'static>(
  cx: &mut Context<V>,
  fetch: impl Future<Output = twitch::Result<T>> + Send + 'static,
  done: impl FnOnce(&mut V, Result<T, String>, &mut Context<V>) + 'static,
  nav: WeakEntity<TwitchSpace>,
) -> Task<()> {
  let fetch = runtime::spawn(cx, fetch);
  cx.spawn(async move |this, cx| {
    let result = fetch.await;
    this
      .update(cx, |view, cx| match result {
        Some(Ok(value)) => done(view, Ok(value), cx),
        Some(Err(e)) => {
          super::report(&nav, &e, cx);
          done(view, Err(super::error_text(&e)), cx);
        }
        None => {}
      })
      .ok();
  })
}

pub struct BrowseView {
  nav: WeakEntity<TwitchSpace>,
  categories: Option<Result<Vec<Category>, String>>,
  _load: Task<()>,
}

impl BrowseView {
  pub fn new(nav: WeakEntity<TwitchSpace>, helix: Helix, cx: &mut Context<Self>) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let task = load(
      cx,
      async move {
        let mut all = Vec::new();
        let mut after = None;
        for _ in 0..CATEGORY_PAGES {
          let page = helix.top_categories(after.as_deref()).await?;
          all.extend(page.items);
          after = page.cursor;
          if after.is_none() {
            break;
          }
        }
        Ok(all)
      },
      |this: &mut Self, result, cx| {
        this.categories = Some(result);
        cx.notify();
      },
      nav.clone(),
    );
    Self {
      nav,
      categories: None,
      _load: task,
    }
  }
}

impl Render for BrowseView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let body = match &self.categories {
      Some(Ok(categories)) => cards::wrap(
        categories
          .iter()
          .map(|c| cards::category_card(c, &self.nav, window, cx))
          .collect(),
      )
      .into_any_element(),
      Some(Err(error)) => cards::note(error.clone()).into_any_element(),
      None => cards::note(t("Loading…", "Chargement…")).into_any_element(),
    };
    page("twitch-browse", t("Browse", "Parcourir"), None, body)
  }
}

pub struct CategoryView {
  nav: WeakEntity<TwitchSpace>,
  name: String,
  streams: Option<Result<(Vec<Stream>, Avatars), String>>,
  _load: Task<()>,
}

impl CategoryView {
  pub fn new(
    nav: WeakEntity<TwitchSpace>,
    helix: Helix,
    id: String,
    name: String,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let task = load(
      cx,
      async move {
        let page = helix.category_streams(&id, None).await?;
        let ids = page.items.iter().map(|s| s.user_id.clone()).collect();
        let avatars = cards::avatars(&helix, ids).await;
        Ok((page.items, avatars))
      },
      |this: &mut Self, result, cx| {
        this.streams = Some(result);
        cx.notify();
      },
      nav.clone(),
    );
    Self {
      nav,
      name,
      streams: None,
      _load: task,
    }
  }
}

impl Render for CategoryView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let width = cards::stream_width(window, cx);
    let body = match &self.streams {
      Some(Ok((streams, _))) if streams.is_empty() => cards::note(t(
        "No one is live in this category.",
        "Personne n'est en direct dans cette catégorie.",
      ))
      .into_any_element(),
      Some(Ok((streams, avatars))) => cards::wrap(
        streams
          .iter()
          .map(|s| cards::stream_card(s, width, avatars.get(&s.user_id), &self.nav, window, cx))
          .collect(),
      )
      .into_any_element(),
      Some(Err(error)) => cards::note(error.clone()).into_any_element(),
      None => cards::note(t("Loading…", "Chargement…")).into_any_element(),
    };
    page(
      "twitch-category",
      &self.name,
      Some(t("Live channels", "Chaînes en direct")),
      body,
    )
  }
}

/// A scrolling page with its title.
pub fn page(
  id: &'static str,
  title: &str,
  subtitle: Option<&str>,
  body: gpui::AnyElement,
) -> impl IntoElement {
  v_flex()
    .id(id)
    .size_full()
    .overflow_y_scroll()
    .p_8()
    .gap_6()
    .child(
      v_flex()
        .gap_1()
        .child(
          div()
            .text_2xl()
            .font_weight(FontWeight::BOLD)
            .child(title.to_owned()),
        )
        .children(subtitle.map(|s| cards::note(s.to_owned()))),
    )
    .child(body)
}
