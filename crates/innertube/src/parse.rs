//! Lenient parsers for InnerTube responses. Shapes verified against `tests/fixtures/`:
//! - grids (home, subscriptions, channel tabs): `richGridRenderer > richItemRenderer >
//!   content > lockupViewModel`, continuation in `continuationItemRenderer`;
//! - related videos: `lockupViewModel` under `secondaryResults`;
//! - watch metadata: `videoPrimaryInfoRenderer` and `videoSecondaryInfoRenderer`.

use serde_json::Value;

use crate::models::{Feed, Thumbnail, VideoItem, WatchPage};

/// Parses a browse response or a browse continuation into a feed.
pub fn parse_feed(response: &Value) -> Feed {
  let mut items = Vec::new();
  collect_lockups(response, &mut items);
  Feed {
    items,
    continuation: find_continuation(response),
    logged_in: tracking_param(response, "logged_in").as_deref() == Some("1"),
    message: find_key(response, "feedNudgeRenderer").map(|nudge| text(&nudge["title"])),
  }
}

/// Parses a `next` response (watch page).
pub fn parse_watch(video_id: &str, response: &Value) -> WatchPage {
  let results = &response["contents"]["twoColumnWatchNextResults"];
  let contents = results["results"]["results"]["contents"]
    .as_array()
    .map(Vec::as_slice)
    .unwrap_or_default();
  let primary = contents
    .iter()
    .find_map(|c| c.get("videoPrimaryInfoRenderer"))
    .unwrap_or(&Value::Null);
  let secondary = contents
    .iter()
    .find_map(|c| c.get("videoSecondaryInfoRenderer"))
    .unwrap_or(&Value::Null);
  let owner = &secondary["owner"]["videoOwnerRenderer"];
  let owner_run = &owner["title"]["runs"][0];

  // The comments section is the item section whose only content is a continuation.
  let comments_continuation = contents
    .iter()
    .filter_map(|c| c.get("itemSectionRenderer"))
    .find_map(find_continuation);

  let secondary_results = &results["secondaryResults"]["secondaryResults"];
  let mut related = Vec::new();
  collect_lockups(secondary_results, &mut related);

  WatchPage {
    id: video_id.to_owned(),
    title: text(&primary["title"]),
    channel: owner_run["text"].as_str().map(str::to_owned),
    channel_id: owner_run["navigationEndpoint"]["browseEndpoint"]["browseId"]
      .as_str()
      .map(str::to_owned),
    channel_avatar: thumbnails(&owner["thumbnail"]["thumbnails"]),
    subscribers: non_empty(text(&owner["subscriberCountText"])),
    views: non_empty(text(
      &primary["viewCount"]["videoViewCountRenderer"]["viewCount"],
    )),
    date: non_empty(text(&primary["dateText"])),
    description: secondary["attributedDescription"]["content"]
      .as_str()
      .unwrap_or_default()
      .to_owned(),
    related,
    related_continuation: find_continuation(secondary_results),
    comments_continuation,
  }
}

/// Collects every video lockup under `value`, skipping malformed ones.
fn collect_lockups(value: &Value, out: &mut Vec<VideoItem>) {
  match value {
    Value::Object(map) => {
      if let Some(lockup) = map.get("lockupViewModel") {
        if let Some(item) = parse_lockup(lockup) {
          out.push(item);
        }
        return;
      }
      map.values().for_each(|v| collect_lockups(v, out));
    }
    Value::Array(values) => values.iter().for_each(|v| collect_lockups(v, out)),
    _ => {}
  }
}

/// One video card. `None` for non-video lockups (playlists, mixes) and malformed items.
fn parse_lockup(lockup: &Value) -> Option<VideoItem> {
  if lockup["contentType"] != "LOCKUP_CONTENT_TYPE_VIDEO" {
    return None;
  }
  let id = lockup["contentId"].as_str()?.to_owned();
  let meta = &lockup["metadata"]["lockupMetadataViewModel"];
  let title = meta["title"]["content"].as_str()?.to_owned();

  // Rows: with two or more, the first is the channel and the next ones views · date; a
  // channel's own tab has a single views · date row.
  let rows: Vec<Vec<String>> = meta["metadata"]["contentMetadataViewModel"]["metadataRows"]
    .as_array()
    .into_iter()
    .flatten()
    .map(|row| {
      row["metadataParts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| part["text"]["content"].as_str().map(str::to_owned))
        .collect()
    })
    .collect();
  let (channel, metadata) = match rows.as_slice() {
    [] => (None, Vec::new()),
    [only] => (None, only.clone()),
    [first, rest @ ..] => (first.first().cloned(), rest.concat()),
  };

  let avatar = &meta["image"]["decoratedAvatarViewModel"];
  let image = &lockup["contentImage"]["thumbnailViewModel"];
  let duration = image["overlays"]
    .as_array()
    .into_iter()
    .flatten()
    .flat_map(|o| {
      o["thumbnailBottomOverlayViewModel"]["badges"]
        .as_array()
        .into_iter()
        .flatten()
    })
    .find_map(|b| b["thumbnailBadgeViewModel"]["text"].as_str())
    .filter(|t| t.contains(':'))
    .map(str::to_owned);

  Some(VideoItem {
    id,
    title,
    channel,
    channel_id: avatar["rendererContext"]["commandContext"]["onTap"]["innertubeCommand"]
      ["browseEndpoint"]["browseId"]
      .as_str()
      .map(str::to_owned),
    metadata,
    duration,
    thumbnails: thumbnails(&image["image"]["sources"]),
    avatar: thumbnails(&avatar["avatar"]["avatarViewModel"]["image"]["sources"]),
  })
}

fn thumbnails(sources: &Value) -> Vec<Thumbnail> {
  sources
    .as_array()
    .into_iter()
    .flatten()
    .filter_map(|s| {
      let url = s["url"].as_str()?;
      // Avatars sometimes come protocol-relative.
      let url = match url.strip_prefix("//") {
        Some(rest) => format!("https://{rest}"),
        None => url.to_owned(),
      };
      Some(Thumbnail {
        url,
        width: s["width"].as_u64().unwrap_or(0) as u32,
        height: s["height"].as_u64().unwrap_or(0) as u32,
      })
    })
    .collect()
}

/// Token of the first `continuationItemRenderer` under `value`.
fn find_continuation(value: &Value) -> Option<String> {
  let item = find_key(value, "continuationItemRenderer")?;
  find_key(&item, "continuationCommand")?["token"]
    .as_str()
    .map(str::to_owned)
}

/// Value of `key` in `responseContext.serviceTrackingParams`.
pub(crate) fn tracking_param(response: &Value, key: &str) -> Option<String> {
  response["responseContext"]["serviceTrackingParams"]
    .as_array()?
    .iter()
    .flat_map(|s| s["params"].as_array().into_iter().flatten())
    .find(|p| p["key"] == key)
    .and_then(|p| p["value"].as_str().map(str::to_owned))
}

/// First value under `key`, depth first.
fn find_key(value: &Value, key: &str) -> Option<Value> {
  match value {
    Value::Object(map) => map
      .get(key)
      .cloned()
      .or_else(|| map.values().find_map(|v| find_key(v, key))),
    Value::Array(values) => values.iter().find_map(|v| find_key(v, key)),
    _ => None,
  }
}

/// Text of a `{simpleText}` or `{runs: [{text}]}` object.
fn text(value: &Value) -> String {
  if let Some(simple) = value["simpleText"].as_str() {
    return simple.to_owned();
  }
  value["runs"]
    .as_array()
    .into_iter()
    .flatten()
    .filter_map(|r| r["text"].as_str())
    .collect()
}

fn non_empty(s: String) -> Option<String> {
  (!s.is_empty()).then_some(s)
}
