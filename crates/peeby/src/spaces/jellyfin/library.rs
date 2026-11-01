//! A library: a virtualized grid of posters, loaded page by page.

use gpui::prelude::*;
use gpui::{
  Context, FontWeight, SharedString, UniformListScrollHandle, WeakEntity, Window, div, px,
  uniform_list,
};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, v_flex};
use jellyfin_api::models::{BaseItem, CollectionType, ItemKind};
use jellyfin_api::{Client, ItemId, ItemsQuery};
use ui::card::poster_height;
use ui::images::Images;
use ui::rows::{GridLayout, Width};
use ui::{runtime, theme};

use super::items::{Shape, card, route_for};
use super::{JellyfinSpace, report};

const PAGE_SIZE: u32 = 200;
const GAP: f32 = 16.;
/// The grid's side padding (`px_8`).
const PADDING: f32 = 32.;
/// Room for the vertical scrollbar.
const SCROLLBAR: f32 = 12.;

pub struct LibraryView {
  client: Client,
  nav: WeakEntity<JellyfinSpace>,
  id: ItemId,
  kinds: Vec<ItemKind>,
  title: SharedString,
  items: Vec<BaseItem>,
  total: Option<u32>,
  loading: bool,
  scroll: UniformListScrollHandle,
  width: Width,
}

/// Item kinds listed for a library type.
pub fn item_types(collection: Option<CollectionType>) -> Vec<ItemKind> {
  match collection {
    Some(CollectionType::Movies) => vec![ItemKind::Movie],
    Some(CollectionType::TvShows) => vec![ItemKind::Series],
    Some(CollectionType::BoxSets) => vec![ItemKind::BoxSet],
    _ => vec![ItemKind::Movie, ItemKind::Series, ItemKind::Video],
  }
}

impl LibraryView {
  pub fn new(
    client: Client,
    nav: WeakEntity<JellyfinSpace>,
    id: ItemId,
    library: Option<BaseItem>,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let title = library
      .as_ref()
      .and_then(|l| l.name.clone())
      .unwrap_or_else(|| "Bibliothèque".into());
    let kinds = item_types(library.as_ref().and_then(|l| l.collection_type));
    let mut view = Self {
      client,
      nav,
      id,
      kinds,
      title: title.into(),
      items: Vec::new(),
      total: None,
      loading: true,
      scroll: UniformListScrollHandle::new(),
      width: Width::default(),
    };
    view.load(true, cx);
    view
  }

  /// Fetches the items again, e.g. after playback. The current grid stays until every page has
  /// arrived, then is replaced at once, so the scroll position holds.
  pub fn reload(&mut self, cx: &mut Context<Self>) {
    self.load(false, cx);
  }

  /// Fetches every page in turn. `progressive` shows each page as it arrives (first load).
  fn load(&mut self, progressive: bool, cx: &mut Context<Self>) {
    let (client, id, kinds) = (self.client.clone(), self.id.clone(), self.kinds.clone());
    cx.spawn(async move |this, cx| {
      let mut start = 0;
      let mut items = Vec::new();
      loop {
        let query = ItemsQuery {
          parent_id: Some(id.clone()),
          include_item_types: kinds.clone(),
          recursive: true,
          sort_by: vec!["SortName".into()],
          start_index: Some(start),
          limit: Some(PAGE_SIZE),
          ..Default::default()
        };
        let Ok(fetch) = this.update(cx, |_, cx| {
          let client = client.clone();
          runtime::spawn(cx, async move { client.items(&query).await })
        }) else {
          return;
        };
        let page = match fetch.await {
          Some(Ok(page)) => page,
          Some(Err(e)) => {
            this
              .update(cx, |this, cx| {
                this.loading = false;
                report(&this.nav, &e, cx);
                cx.notify();
              })
              .ok();
            return;
          }
          None => return,
        };
        let received = page.items.len() as u32;
        let total = page.total_record_count;
        let alive = if progressive {
          this.update(cx, |this, cx| {
            this.items.extend(page.items);
            this.total = Some(total);
            cx.notify();
          })
        } else {
          items.extend(page.items);
          Ok(())
        };
        start += received;
        if alive.is_err() || received == 0 || start >= total {
          if !progressive {
            this
              .update(cx, |this, _| {
                this.items = std::mem::take(&mut items);
                this.total = Some(total);
              })
              .ok();
          }
          break;
        }
      }
      this
        .update(cx, |this, cx| {
          this.loading = false;
          cx.notify();
        })
        .ok();
    })
    .detach();
  }
}

impl Render for LibraryView {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let shape = Shape::Poster;
    // The page's own width (the shell's sidebar takes the rest of the window), minus the
    // padding and the scrollbar; one column until it is measured.
    let available = (self.width.get() - 2. * PADDING - SCROLLBAR).max(shape.width());
    let layout = GridLayout::new(available, shape.width(), GAP);
    let rows = layout.rows(self.items.len());
    let count = match (self.total, self.loading) {
      (Some(1), _) => "1 titre".to_owned(),
      (Some(total), _) => format!("{total} titres"),
      (None, true) => "Chargement…".into(),
      (None, false) => String::new(),
    };

    let grid = uniform_list(
      "library-grid",
      rows,
      cx.processor(move |this, range: std::ops::Range<usize>, window, cx| {
        range
          .map(|row| {
            let start = row * layout.columns;
            let end = (start + layout.columns).min(this.items.len());
            let cards: Vec<_> = this.items[start..end]
              .iter()
              .map(|item| {
                card(
                  "grid",
                  item,
                  shape,
                  route_for(item),
                  &this.nav,
                  &this.client,
                  window,
                  cx,
                )
              })
              .collect();
            h_flex()
              .h(px(poster_height(shape.width(), shape.aspect()) + GAP))
              .gap(px(layout.gap))
              .items_start()
              .children(cards)
          })
          .collect()
      }),
    )
    .track_scroll(&self.scroll)
    .size_full()
    .px_8();

    v_flex()
      .size_full()
      .relative()
      .child(self.width.probe())
      .child(
        h_flex()
          .px_8()
          .pt_8()
          .pb_4()
          .gap_3()
          .items_end()
          .child(
            div()
              .text_size(px(28.))
              .font_weight(FontWeight::BOLD)
              .child(self.title.clone()),
          )
          .child(
            div()
              .pb_1()
              .text_sm()
              .text_color(theme::muted())
              .child(count),
          ),
      )
      .child(
        div()
          .flex_1()
          .min_h_0()
          .relative()
          .child(grid)
          .vertical_scrollbar(&self.scroll),
      )
  }
}
