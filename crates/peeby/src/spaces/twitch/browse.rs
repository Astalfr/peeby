//! "Parcourir": the categories with the most viewers (`games`), and a category's page with its
//! live streams, most viewers first: a hundred of each, GQL's first page.

use gpui::prelude::*;
use gpui::{Context, FontWeight, Task, Window, div};
use gpui_component::v_flex;
use twitch::catalog::Catalog;
use twitch::helix::{Category, Stream};
use ui::i18n::t;
use ui::images::Images;
use ui::runtime;

use super::Nav;
use super::cards::{self, Avatars};

/// Loads `fetch` on the network runtime and keeps its result in the view.
pub(super) fn load<V: 'static, T: Send + 'static>(
  cx: &mut Context<V>,
  fetch: impl Future<Output = twitch::Result<T>> + Send + 'static,
  done: impl FnOnce(&mut V, Result<T, String>, &mut Context<V>) + 'static,
  nav: Nav,
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
  nav: Nav,
  categories: Option<Result<Vec<Category>, String>>,
  _load: Task<()>,
}

impl BrowseView {
  pub fn new(nav: Nav, catalog: Catalog, cx: &mut Context<Self>) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let task = load(
      cx,
      async move { catalog.top_categories().await },
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
  nav: Nav,
  name: String,
  streams: Option<Result<(Vec<Stream>, Avatars), String>>,
  _load: Task<()>,
}

impl CategoryView {
  pub fn new(nav: Nav, catalog: Catalog, id: String, name: String, cx: &mut Context<Self>) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let task = load(
      cx,
      async move {
        let streams = catalog.category_streams(&id).await?;
        let ids = streams.iter().map(|s| s.user_id.clone()).collect();
        let avatars = cards::avatars(&catalog, ids).await;
        Ok((streams, avatars))
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
