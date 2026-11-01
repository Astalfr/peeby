//! Playback extras: media segments (intro, outro…), trickplay thumbnails, previous and next
//! episodes, and the watched flag.

use std::collections::HashMap;

use reqwest::Method;
use serde::Deserialize;
use url::Url;

use crate::models::{BaseItem, QueryResult};
use crate::{Client, Error, ItemId, Result};

/// The neighbours of an episode, from `GET /Shows/{seriesId}/Episodes?adjacentTo=…`.
#[derive(Debug, Default)]
pub struct Adjacent {
  /// The episode before, none for the first one.
  pub previous: Option<BaseItem>,
  /// The episode after, none for the last one.
  pub next: Option<BaseItem>,
}

impl Adjacent {
  /// The server returns up to three items in series order: previous, `episode`, next. The first
  /// episode has no previous one and the last no next one, so sides are found by position.
  fn around(mut items: Vec<BaseItem>, episode: &ItemId) -> Self {
    let Some(at) = items.iter().position(|e| &e.id == episode) else {
      return Self::default();
    };
    let next = items.drain(at..).nth(1);
    Self {
      previous: items.pop(),
      next,
    }
  }
}

/// 1 tick = 100 ns.
pub const TICKS_PER_SECOND: f64 = 10_000_000.0;

/// `MediaSegmentType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum SegmentKind {
  /// Advertisement.
  Commercial,
  /// Preview of the next episode.
  Preview,
  /// "Previously on".
  Recap,
  /// End credits.
  Outro,
  /// Opening.
  Intro,
  /// Unknown, or a type added later.
  #[serde(other)]
  Unknown,
}

/// `MediaSegmentDto`, reduced.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaSegment {
  /// Segment type.
  #[serde(rename = "Type")]
  pub kind: SegmentKind,
  /// Start, in ticks.
  pub start_ticks: i64,
  /// End, in ticks.
  pub end_ticks: i64,
}

impl MediaSegment {
  /// Start in seconds.
  pub fn start_seconds(&self) -> f64 {
    self.start_ticks as f64 / TICKS_PER_SECOND
  }

  /// End in seconds.
  pub fn end_seconds(&self) -> f64 {
    self.end_ticks as f64 / TICKS_PER_SECOND
  }
}

/// `TrickplayInfoDto`: one resolution of trickplay thumbnails, packed in tile images.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct TrickplayInfo {
  /// Width of one thumbnail in pixels (also the key of this resolution).
  pub width: u32,
  /// Height of one thumbnail in pixels.
  pub height: u32,
  /// Thumbnails per tile row.
  pub tile_width: u32,
  /// Thumbnails per tile column.
  pub tile_height: u32,
  /// Number of thumbnails.
  pub thumbnail_count: u32,
  /// Milliseconds between two thumbnails.
  pub interval: u32,
}

/// Where one thumbnail is: tile image index and pixel offset inside that tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrickplayCell {
  /// Tile image index (`/Trickplay/{width}/{tile}.jpg`).
  pub tile: u32,
  /// Left edge of the thumbnail in the tile, pixels.
  pub x: u32,
  /// Top edge of the thumbnail in the tile, pixels.
  pub y: u32,
}

impl TrickplayInfo {
  /// The thumbnail shown at `seconds`, or `None` without thumbnails.
  pub fn cell_at(&self, seconds: f64) -> Option<TrickplayCell> {
    let per_tile = self.tile_width * self.tile_height;
    if self.interval == 0 || per_tile == 0 || self.thumbnail_count == 0 {
      return None;
    }
    let thumb = ((seconds.max(0.0) * 1000.0) as u64 / u64::from(self.interval))
      .min(u64::from(self.thumbnail_count - 1)) as u32;
    let in_tile = thumb % per_tile;
    Some(TrickplayCell {
      tile: thumb / per_tile,
      x: in_tile % self.tile_width * self.width,
      y: in_tile / self.tile_width * self.height,
    })
  }

  /// Size of a full tile image, pixels.
  pub fn tile_size(&self) -> (u32, u32) {
    (self.width * self.tile_width, self.height * self.tile_height)
  }
}

/// `BaseItemDto.Trickplay`: media source id → thumbnail width → info.
pub type TrickplayManifest = HashMap<String, HashMap<String, TrickplayInfo>>;

/// For a media source, the smallest resolution at least `min_width` wide, else the largest.
pub fn pick_trickplay<'a>(
  manifest: &'a TrickplayManifest,
  media_source_id: &str,
  min_width: u32,
) -> Option<&'a TrickplayInfo> {
  let mut infos: Vec<&TrickplayInfo> = manifest.get(media_source_id)?.values().collect();
  infos.sort_by_key(|i| i.width);
  infos
    .iter()
    .find(|i| i.width >= min_width)
    .or(infos.last())
    .copied()
}

impl Client {
  /// `GET /MediaSegments/{itemId}`. A server without segments, or without the endpoint
  /// (added in 10.10), yields an empty list.
  pub async fn media_segments(&self, item: &ItemId) -> Result<Vec<MediaSegment>> {
    let endpoint = format!("MediaSegments/{item}");
    let request = self.request(Method::GET, &endpoint)?;
    match self
      .send_json::<QueryResult<MediaSegment>>(request, &endpoint)
      .await
    {
      Ok(result) => Ok(result.items),
      Err(Error::Status { status: 404, .. }) => Ok(Vec::new()),
      Err(e) => Err(e),
    }
  }

  /// The episodes before and after `episode` in the series order (across seasons), skipping
  /// missing ones.
  pub async fn adjacent_episodes(&self, series: &ItemId, episode: &ItemId) -> Result<Adjacent> {
    let endpoint = format!("Shows/{series}/Episodes");
    let user = self.user_id()?;
    let query = [
      ("userId", user.to_string()),
      ("adjacentTo", episode.to_string()),
      ("isMissing", "false".into()),
      ("enableUserData", "true".into()),
    ];
    let request = self.request(Method::GET, &endpoint)?.query(&query);
    let result: QueryResult<BaseItem> = self.send_json(request, &endpoint).await?;
    Ok(Adjacent::around(result.items, episode))
  }

  /// `POST /UserPlayedItems/{itemId}`: marks an item watched.
  pub async fn mark_played(&self, item: &ItemId) -> Result<()> {
    let endpoint = format!("UserPlayedItems/{item}");
    let user = self.user_id()?;
    let request = self
      .request(Method::POST, &endpoint)?
      .query(&[("userId", user.as_str())]);
    self.send_empty(request, &endpoint).await
  }

  /// URL of one trickplay tile image (JPEG).
  pub fn trickplay_tile_url(
    &self,
    item: &ItemId,
    media_source_id: &str,
    width: u32,
    tile: u32,
  ) -> Result<Url> {
    let mut url = self
      .base_url()
      .join(&format!("Videos/{item}/Trickplay/{width}/{tile}.jpg"))?;
    url
      .query_pairs_mut()
      .append_pair("mediaSourceId", media_source_id);
    Ok(url)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn info() -> TrickplayInfo {
    TrickplayInfo {
      width: 320,
      height: 180,
      tile_width: 10,
      tile_height: 10,
      thumbnail_count: 250,
      interval: 10_000,
    }
  }

  #[test]
  fn trickplay_cells() {
    let info = info();
    let cell = |tile, x, y| Some(TrickplayCell { tile, x, y });
    assert_eq!(info.cell_at(0.0), cell(0, 0, 0));
    // Thumbnail 13: first tile, row 1, column 3.
    assert_eq!(info.cell_at(135.0), cell(0, 960, 180));
    // Thumbnail 104: second tile, row 0, column 4.
    assert_eq!(info.cell_at(1045.0), cell(1, 1280, 0));
    // Past the end: the last thumbnail (249: third tile, row 4, column 9).
    assert_eq!(info.cell_at(99_999.0), cell(2, 2880, 720));
    assert_eq!(info.tile_size(), (3200, 1800));
  }

  #[test]
  fn trickplay_without_thumbnails() {
    let info = TrickplayInfo {
      thumbnail_count: 0,
      ..info()
    };
    assert_eq!(info.cell_at(10.0), None);
  }

  #[test]
  fn trickplay_manifest_and_segments_parse() {
    let manifest: TrickplayManifest = serde_json::from_value(serde_json::json!({
      "src": {
        "320": { "Width": 320, "Height": 180, "TileWidth": 10, "TileHeight": 10,
                 "ThumbnailCount": 250, "Interval": 10000, "Bandwidth": 1 },
        "640": { "Width": 640, "Height": 360, "TileWidth": 10, "TileHeight": 10,
                 "ThumbnailCount": 250, "Interval": 10000, "Bandwidth": 1 }
      }
    }))
    .unwrap();
    assert_eq!(pick_trickplay(&manifest, "src", 300).unwrap().width, 320);
    assert_eq!(pick_trickplay(&manifest, "src", 400).unwrap().width, 640);
    assert_eq!(pick_trickplay(&manifest, "src", 1000).unwrap().width, 640);
    assert!(pick_trickplay(&manifest, "other", 300).is_none());

    let segments: QueryResult<MediaSegment> = serde_json::from_value(serde_json::json!({
      "Items": [
        { "Id": "a", "ItemId": "b", "Type": "Intro", "StartTicks": 0, "EndTicks": 900000000 },
        { "Id": "c", "ItemId": "b", "Type": "Something", "StartTicks": 1, "EndTicks": 2 }
      ],
      "TotalRecordCount": 2, "StartIndex": 0
    }))
    .unwrap();
    assert_eq!(segments.items[0].kind, SegmentKind::Intro);
    assert_eq!(segments.items[0].end_seconds(), 90.0);
    assert_eq!(segments.items[1].kind, SegmentKind::Unknown);
  }

  #[test]
  fn adjacent_by_position() {
    let items = |ids: &[&str]| -> Vec<BaseItem> {
      ids
        .iter()
        .map(|id| {
          serde_json::from_value(serde_json::json!({ "Id": id, "Type": "Episode" })).unwrap()
        })
        .collect()
    };
    let id = |item: Option<BaseItem>| item.map(|i| i.id.0);
    let current = ItemId::from("b");

    let middle = Adjacent::around(items(&["a", "b", "c"]), &current);
    assert_eq!(id(middle.previous).as_deref(), Some("a"));
    assert_eq!(id(middle.next).as_deref(), Some("c"));

    // First episode: the item after it is not a previous one.
    let first = Adjacent::around(items(&["b", "c"]), &current);
    assert_eq!(id(first.previous), None);
    assert_eq!(id(first.next).as_deref(), Some("c"));

    let last = Adjacent::around(items(&["a", "b"]), &current);
    assert_eq!(id(last.previous).as_deref(), Some("a"));
    assert_eq!(id(last.next), None);

    let missing = Adjacent::around(items(&["a", "c"]), &current);
    assert!(missing.previous.is_none() && missing.next.is_none());
  }
}
