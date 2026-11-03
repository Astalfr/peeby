//! Jellyfin items as the shared components show them: which image of which item a card, a
//! banner or a logo shows ([`Picture`]s sized by the server, with their BlurHash), the cards
//! themselves, where a click leads, and the texts (episode numbers, durations).

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{AnyElement, App, FontWeight, SharedString, Window, div};
use jellyfin_api::models::{BaseItem, ImageType, ItemKind};
use jellyfin_api::{Client, ImageOptions, ItemId};
use ui::card::{Corner, Hover, MediaCard, TALL, WIDE};
use ui::picture::Picture;
use ui::tf;

use super::{Nav, Route};
use crate::settings::Settings;

/// Card shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
  /// 2:3 poster (movies, series, seasons).
  Poster,
  /// Larger 2:3 poster (home "Derniers ajouts" rows).
  LargePoster,
  /// 16:9 thumbnail (episodes, "Reprendre").
  Thumb,
}

impl Shape {
  pub fn width(self) -> f32 {
    match self {
      Self::Poster => 160.,
      Self::LargePoster => 192.,
      Self::Thumb => 288.,
    }
  }

  pub fn aspect(self) -> f32 {
    match self {
      Self::Poster | Self::LargePoster => TALL,
      Self::Thumb => WIDE,
    }
  }
}

/// Which image of which item to show.
#[derive(Clone, Debug)]
pub struct ImageRef {
  pub item: ItemId,
  pub kind: ImageType,
  pub tag: Option<String>,
  pub blurhash: Option<String>,
  pub index: Option<u32>,
  /// Ask the server for a blurred image (spoiler-free stills).
  pub blur: bool,
}

impl ImageRef {
  fn new(item: ItemId, kind: ImageType, tag: Option<String>) -> Self {
    Self {
      item,
      kind,
      tag,
      blurhash: None,
      index: None,
      blur: false,
    }
  }

  /// The image filling a box, as WebP at quality 85, with its BlurHash meanwhile.
  pub fn picture(self, client: &Client) -> Picture {
    let blurhash = self.blurhash.clone().map(SharedString::from);
    let client = client.clone();
    Picture::Sized {
      url: Rc::new(move |width, height| {
        let options = ImageOptions {
          fill: Some((width, height)),
          tag: self.tag.clone(),
          index: self.index,
          blur: self.blur.then_some(40),
          format: Some("Webp"),
          quality: Some(85),
          ..Default::default()
        };
        image_url(&client, &self, &options)
      }),
      blurhash,
    }
  }

  /// A transparent logo, scaled down to fit a box (never cropped).
  pub fn logo(self, client: &Client) -> Picture {
    let client = client.clone();
    Picture::sized(move |width, height| {
      let options = ImageOptions {
        max_width: Some(width),
        max_height: Some(height),
        tag: self.tag.clone(),
        format: Some("Webp"),
        quality: Some(90),
        ..Default::default()
      };
      image_url(&client, &self, &options)
    })
  }
}

/// The URL of an image; an empty one (which fails to load, leaving the placeholder) if the
/// server address cannot hold it.
fn image_url(client: &Client, image: &ImageRef, options: &ImageOptions) -> String {
  client
    .image_url(&image.item, image.kind, options)
    .map(|url| url.to_string())
    .unwrap_or_default()
}

/// The image of `item` for a card of `shape`.
pub fn image_ref(item: &BaseItem, shape: Shape) -> Option<ImageRef> {
  let own = |kind: ImageType| {
    item.image_tag(kind).map(|tag| ImageRef {
      blurhash: item.blur_hash(kind).map(str::to_owned),
      ..ImageRef::new(item.id.clone(), kind, Some(tag.to_owned()))
    })
  };
  match shape {
    Shape::Poster | Shape::LargePoster => {
      let series_poster = || {
        Some(ImageRef::new(
          item.series_id.clone()?,
          ImageType::Primary,
          item.series_primary_image_tag.clone(),
        ))
      };
      // An episode's own primary image is a 16:9 still: the series poster instead.
      if item.kind == ItemKind::Episode {
        series_poster().or_else(|| own(ImageType::Primary))
      } else {
        own(ImageType::Primary).or_else(series_poster)
      }
    }
    Shape::Thumb => {
      if item.kind == ItemKind::Episode {
        own(ImageType::Primary).or_else(|| parent_thumb(item))
      } else {
        own(ImageType::Thumb)
          .or_else(|| backdrop_ref(item))
          .or_else(|| own(ImageType::Primary))
      }
    }
  }
}

fn parent_thumb(item: &BaseItem) -> Option<ImageRef> {
  Some(ImageRef::new(
    item.parent_thumb_item_id.clone()?,
    ImageType::Thumb,
    item.parent_thumb_image_tag.clone(),
  ))
}

/// The item's logo, or its series' logo for an episode.
pub fn logo_ref(item: &BaseItem) -> Option<ImageRef> {
  if let Some(tag) = item.image_tag(ImageType::Logo) {
    return Some(ImageRef::new(
      item.id.clone(),
      ImageType::Logo,
      Some(tag.to_owned()),
    ));
  }
  Some(ImageRef::new(
    item.parent_logo_item_id.clone()?,
    ImageType::Logo,
    item.parent_logo_image_tag.clone(),
  ))
}

/// The item's first backdrop, or its parent's (the series' for episodes and seasons).
pub fn backdrop_ref(item: &BaseItem) -> Option<ImageRef> {
  if let Some(tag) = item.backdrop_image_tags.as_ref().and_then(|t| t.first()) {
    return Some(ImageRef {
      blurhash: item
        .image_blur_hashes
        .as_ref()
        .and_then(|m| m.get("Backdrop"))
        .and_then(|m| m.get(tag))
        .cloned()
        .flatten(),
      index: Some(0),
      ..ImageRef::new(item.id.clone(), ImageType::Backdrop, Some(tag.clone()))
    });
  }
  Some(ImageRef {
    index: Some(0),
    ..ImageRef::new(
      item.parent_backdrop_item_id.clone()?,
      ImageType::Backdrop,
      item.parent_backdrop_image_tags.as_ref()?.first().cloned(),
    )
  })
}

/// Whether "hide spoilers" applies to an item: an episode not watched yet.
pub fn is_spoiler(item: &BaseItem, cx: &App) -> bool {
  item.kind == ItemKind::Episode
    && !item.user_data.as_ref().is_some_and(|u| u.played)
    && Settings::get(cx).jellyfin.home.hide_spoilers
}

/// Blurs an episode's own still (not series artwork) when it would spoil.
pub fn spoiler_safe(mut image: ImageRef, item: &BaseItem, cx: &App) -> ImageRef {
  if image.item == item.id && is_spoiler(item, cx) {
    image.blur = true;
    image.blurhash = None;
  }
  image
}

/// The picture of `item` for `shape`, blurred when it would spoil.
pub fn card_picture(client: &Client, item: &BaseItem, shape: Shape, cx: &App) -> Picture {
  match image_ref(item, shape) {
    Some(image) => spoiler_safe(image, item, cx).picture(client),
    None => Picture::Sizes(Vec::new()),
  }
}

/// How far an item was watched, 0 to 1, when it is partly watched (0.5 % to 99.5 %).
pub fn progress(item: &BaseItem) -> Option<f32> {
  item
    .user_data
    .as_ref()
    .and_then(|u| u.played_percentage)
    .filter(|p| *p > 0.5 && *p < 99.5)
    .map(|p| (p / 100.) as f32)
}

/// Whether the item has a position to resume from.
pub fn resumable(item: &BaseItem) -> bool {
  item
    .user_data
    .as_ref()
    .is_some_and(|u| u.playback_position_ticks > 0 && !u.played)
}

/// A card for `item`: a click opens `route` (its page, or playback with a play button on hover).
/// `scope` tells apart cards of the same item on one page (two home rows). On an episode card
/// that plays, the series name is a link of its own to the series page.
#[allow(clippy::too_many_arguments)] // One card needs all of these; a struct would only rename them.
pub fn card(
  scope: &str,
  item: &BaseItem,
  shape: Shape,
  route: Route,
  nav: &Nav,
  client: &Client,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let id = format!("card-{scope}-{}", item.id);
  let hover = Hover::new(&id, window, cx);
  let (title, line) = card_text(item);
  let data = item.user_data.as_ref();
  let corner = match (data.and_then(|u| u.unplayed_item_count), data) {
    (Some(n), _) if n > 0 => Some(Corner::Count(n.unsigned_abs())),
    (_, Some(u)) if u.played => Some(Corner::Watched),
    _ => None,
  };
  let plays = matches!(route, Route::Play(_));
  let series_route =
    (plays && item.kind == ItemKind::Episode && item.series_id.is_some()).then(|| route_for(item));
  let (open, in_new_tab) = (nav.clone(), nav.clone());
  let label = SharedString::from(title.clone());
  let new_route = route.clone();
  let mut card = MediaCard::new(id, hover, title, card_picture(client, item, shape, cx))
    .lines([line])
    .progress(progress(item))
    .corner(corner)
    .play(plays)
    .on_click(move |window, cx| open.open(route.clone(), window, cx))
    .on_middle_click(move |_, cx| {
      in_new_tab.open_in_new_tab(new_route.clone(), Some(label.clone()), cx)
    });
  if let Some(series) = series_route {
    let (open, in_new_tab) = (nav.clone(), nav.clone());
    let label = item.series_name.clone().map(SharedString::from);
    let new_route = series.clone();
    card = card
      .on_title_click(move |window, cx| open.open(series.clone(), window, cx))
      .on_title_middle_click(move |_, cx| {
        in_new_tab.open_in_new_tab(new_route.clone(), label.clone(), cx)
      });
  }
  card.poster(shape.width(), shape.aspect(), window, cx)
}

/// A card's title and the line under it: the series and "S1:E2 · name" for an episode, else the
/// name and the year.
fn card_text(item: &BaseItem) -> (String, String) {
  let name = item.name.clone().unwrap_or_default();
  match item.kind {
    ItemKind::Episode => (
      item.series_name.clone().unwrap_or_default(),
      format!("{}{name}", episode_number(item)),
    ),
    _ => (
      name,
      item
        .production_year
        .map(|y| y.to_string())
        .unwrap_or_default(),
    ),
  }
}

/// `S1:E2 · `, `E2 · ` or nothing.
pub fn episode_number(item: &BaseItem) -> String {
  match (item.parent_index_number, item.index_number) {
    (Some(s), Some(e)) => format!("S{s}:E{e} · "),
    (None, Some(e)) => format!("E{e} · "),
    _ => String::new(),
  }
}

/// Where clicking an item leads: an episode or a season opens its series on that season.
pub fn route_for(item: &BaseItem) -> Route {
  match (item.kind, &item.series_id) {
    (ItemKind::Episode | ItemKind::Season, Some(series)) => Route::Item {
      id: series.clone(),
      season: item
        .season_id
        .clone()
        .or_else(|| (item.kind == ItemKind::Season).then(|| item.id.clone())),
    },
    _ => Route::Item {
      id: item.id.clone(),
      season: None,
    },
  }
}

/// Ticks (100 ns) per second.
pub const TICKS_PER_SECOND: i64 = 10_000_000;

/// `1 h 42 min` (French: `1 h 42`), `48 min`.
pub fn runtime(ticks: i64) -> String {
  let minutes = ticks / (60 * TICKS_PER_SECOND);
  match (minutes / 60, minutes % 60) {
    (0, m) => format!("{m} min"),
    (h, 0) => format!("{h} h"),
    (h, m) => tf!("{h} h {m:02} min", "{h} h {m:02}"),
  }
}

/// `12:34` or `1:02:03` from ticks.
pub fn clock(ticks: i64) -> String {
  ui::player::format_time((ticks.max(0) / TICKS_PER_SECOND) as f64)
}

/// A section title (home rows, the episodes of a series).
pub fn section_title(text: impl Into<SharedString>) -> impl IntoElement {
  div()
    .text_lg()
    .font_weight(FontWeight::SEMIBOLD)
    .child(text.into())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn durations() {
    assert_eq!(runtime(23 * 60 * TICKS_PER_SECOND), "23 min");
    assert_eq!(runtime(120 * 60 * TICKS_PER_SECOND), "2 h");
    assert_eq!(runtime(102 * 60 * TICKS_PER_SECOND), "1 h 42 min");
    assert_eq!(clock(754 * TICKS_PER_SECOND), "12:34");
  }
}
