//! Lenient parsers for InnerTube responses. Shapes verified against `tests/fixtures/`:
//! - grids (home, subscriptions, channel tabs): `richGridRenderer > richItemRenderer >
//!   content > lockupViewModel`, continuation in `continuationItemRenderer`;
//! - related videos: `lockupViewModel` under `secondaryResults`;
//! - watch metadata: `videoPrimaryInfoRenderer` and `videoSecondaryInfoRenderer`.

use serde_json::Value;

use std::collections::HashMap;

use crate::models::{Channel, Comment, CommentPage, Feed, Thumbnail, VideoItem, WatchPage};

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

/// Parses the subscribed channels list (`FEchannels`: all channels in one response).
pub fn parse_channels(response: &Value) -> Vec<Channel> {
  let mut renderers = Vec::new();
  find_all(response, "channelRenderer", &mut renderers);
  renderers
    .into_iter()
    .filter_map(|c| {
      Some(Channel {
        id: c["channelId"].as_str()?.to_owned(),
        name: text(&c["title"]).trim().to_owned(),
        // In this list YouTube puts the handle in `subscriberCountText` and the subscriber
        // count in `videoCountText`.
        handle: non_empty(text(&c["subscriberCountText"])).filter(|h| h.starts_with('@')),
        subscribers: non_empty(text(&c["videoCountText"])),
        avatar: thumbnails(&c["thumbnail"]["thumbnails"]),
      })
    })
    .collect()
}

/// Parses a comments page (`next` continuation). Threads are `commentThreadRenderer`s pointing
/// by key to `commentEntityPayload` mutations; replies pages list `commentViewModel`s.
pub fn parse_comments(response: &Value) -> CommentPage {
  let entities: HashMap<&str, &Value> =
    response["frameworkUpdates"]["entityBatchUpdate"]["mutations"]
      .as_array()
      .into_iter()
      .flatten()
      .filter_map(|m| {
        let payload = m["payload"].get("commentEntityPayload")?;
        Some((m["entityKey"].as_str()?, payload))
      })
      .collect();
  let mut page = CommentPage::default();
  let items = response["onResponseReceivedEndpoints"]
    .as_array()
    .into_iter()
    .flatten()
    .filter_map(|e| {
      e.get("reloadContinuationItemsCommand")
        .or_else(|| e.get("appendContinuationItemsAction"))
    })
    .flat_map(|c| c["continuationItems"].as_array().into_iter().flatten());
  for item in items {
    if let Some(header) = item.get("commentsHeaderRenderer") {
      page.count = non_empty(text(&header["countText"]));
    } else if let Some(cont) = item.get("continuationItemRenderer") {
      page.continuation =
        find_key(cont, "continuationCommand").and_then(|c| c["token"].as_str().map(str::to_owned));
    } else {
      let (view, replies) = match item.get("commentThreadRenderer") {
        Some(thread) => (
          &thread["commentViewModel"]["commentViewModel"],
          &thread["replies"],
        ),
        None => (&item["commentViewModel"], &Value::Null),
      };
      let Some(entity) = view["commentKey"].as_str().and_then(|k| entities.get(k)) else {
        continue;
      };
      if let Some(mut comment) = parse_comment(entity) {
        comment.replies_continuation = find_continuation(replies);
        page.comments.push(comment);
      }
    }
  }
  page
}

fn parse_comment(entity: &Value) -> Option<Comment> {
  let properties = &entity["properties"];
  let author = &entity["author"];
  let toolbar = &entity["toolbar"];
  let string = |v: &Value| v.as_str().unwrap_or_default().trim().to_owned();
  Some(Comment {
    id: properties["commentId"].as_str()?.to_owned(),
    author: string(&author["displayName"]),
    avatar: author["avatarThumbnailUrl"].as_str().map(str::to_owned),
    by_creator: author["isCreator"].as_bool().unwrap_or(false),
    text: properties["content"]["content"].as_str()?.to_owned(),
    published: string(&properties["publishedTime"]),
    likes: string(&toolbar["likeCountNotliked"]),
    replies: string(&toolbar["replyCount"]),
    replies_continuation: None,
  })
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

/// Every value under `key`, depth first (not looking inside matches).
fn find_all<'a>(value: &'a Value, key: &str, out: &mut Vec<&'a Value>) {
  match value {
    Value::Object(map) => match map.get(key) {
      Some(found) => out.push(found),
      None => map.values().for_each(|v| find_all(v, key, out)),
    },
    Value::Array(values) => values.iter().for_each(|v| find_all(v, key, out)),
    _ => {}
  }
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
