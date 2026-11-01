//! Lenient parsers for InnerTube responses. Shapes verified against `tests/fixtures/`:
//! - grids (home, subscriptions, channel tabs): `richGridRenderer > richItemRenderer >
//!   content > lockupViewModel`, continuation in `continuationItemRenderer`;
//! - related videos: `lockupViewModel` under `secondaryResults`;
//! - watch metadata: `videoPrimaryInfoRenderer` and `videoSecondaryInfoRenderer`, the rating
//!   in its `segmentedLikeDislikeButtonViewModel`;
//! - the playlists library: playlist `lockupViewModel`s with a `collectionThumbnailViewModel`;
//! - a playlist's videos: video `lockupViewModel`s, or `playlistVideoRenderer`s for "Watch
//!   later", next to a `continuationItemViewModel`; its header is a `playlistHeaderRenderer`
//!   ("Watch later", "Liked videos") or a `pageHeaderViewModel` (other playlists);
//! - a subscribed channel's bell: `subscriptionNotificationToggleButtonRenderer` in the
//!   `subscribeButtonRenderer` of the watch page's owner and of `FEchannels` entries.

use serde_json::Value;

use std::collections::HashMap;

use crate::models::{
  Caption, Channel, ChannelNotifications, Comment, CommentPage, Feed, LikeParams, LikeStatus,
  NotificationLevel, Playlist, PlaylistChoice, PlaylistSummary, Thumbnail, VideoItem, WatchPage,
};

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

  let (like_status, like_params) = parse_like_buttons(&primary["videoActions"]);

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
    like_status,
    like_params,
    notifications: parse_notification_bell(&secondary["subscribeButton"]),
  }
}

/// The notification bell of a subscribe button: `None` signed out or not subscribed (the
/// button then carries a bell anyway, for after a subscription). The current level is the icon
/// of the state `currentStateId` names; the options are the menu items calling
/// `modifyChannelNotificationPreferenceEndpoint` (not "Se désabonner").
fn parse_notification_bell(subscribe_button: &Value) -> Option<ChannelNotifications> {
  let button = &subscribe_button["subscribeButtonRenderer"];
  if button["subscribed"] != true {
    return None;
  }
  let toggle = find_key(button, "subscriptionNotificationToggleButtonRenderer")?;
  let level = toggle["states"]
    .as_array()?
    .iter()
    .find(|s| s["stateId"] == toggle["currentStateId"])
    .and_then(|s| notification_level(&s["state"]["buttonRenderer"]["icon"]))?;
  let mut items = Vec::new();
  find_all(&toggle["command"], "menuServiceItemRenderer", &mut items);
  let options = items
    .into_iter()
    .filter_map(|item| {
      let params = item["serviceEndpoint"]["modifyChannelNotificationPreferenceEndpoint"]["params"]
        .as_str()?;
      Some((notification_level(&item["icon"])?, params.to_owned()))
    })
    .collect::<Vec<_>>();
  (!options.is_empty()).then_some(ChannelNotifications { level, options })
}

/// The level a bell icon stands for.
fn notification_level(icon: &Value) -> Option<NotificationLevel> {
  match icon["iconType"].as_str()? {
    "NOTIFICATIONS_ACTIVE" => Some(NotificationLevel::All),
    "NOTIFICATIONS_NONE" => Some(NotificationLevel::Personalized),
    "NOTIFICATIONS_OFF" => Some(NotificationLevel::None),
    _ => None,
  }
}

/// The user's rating and the params of the like and dislike buttons
/// (`segmentedLikeDislikeButtonViewModel`). Signed out, the endpoints sit inside a sign-in
/// modal, so they are searched for rather than read at a fixed path.
fn parse_like_buttons(actions: &Value) -> (LikeStatus, LikeParams) {
  let Some(buttons) = find_key(actions, "segmentedLikeDislikeButtonViewModel") else {
    return Default::default();
  };
  let status = match find_key(&buttons, "likeStatusEntity")
    .as_ref()
    .and_then(|e| e["likeStatus"].as_str())
  {
    Some("LIKE") => LikeStatus::Like,
    Some("DISLIKE") => LikeStatus::Dislike,
    _ => LikeStatus::Indifferent,
  };
  let mut endpoints = Vec::new();
  find_all(&buttons, "likeEndpoint", &mut endpoints);
  let mut params = LikeParams::default();
  for endpoint in endpoints {
    let (slot, key) = match endpoint["status"].as_str() {
      Some("LIKE") => (&mut params.like, "likeParams"),
      Some("DISLIKE") => (&mut params.dislike, "dislikeParams"),
      Some("INDIFFERENT") => (&mut params.remove, "removeLikeParams"),
      _ => continue,
    };
    if slot.is_none() {
      *slot = endpoint[key].as_str().map(str::to_owned);
    }
  }
  (status, params)
}

/// Subtitle tracks of a player response, as WebVTT URLs. Machine translations (`tlang`) are
/// not offered: YouTube answers them with HTTP 429 for this client (checked 2026-11-01).
pub fn parse_captions(player: &Value) -> Vec<Caption> {
  player["captions"]["playerCaptionsTracklistRenderer"]["captionTracks"]
    .as_array()
    .into_iter()
    .flatten()
    .filter_map(|t| {
      Some(Caption {
        language: t["languageCode"].as_str()?.to_owned(),
        name: text(&t["name"]),
        auto_generated: t["kind"] == "asr",
        url: format!("{}&fmt=vtt", t["baseUrl"].as_str()?),
      })
    })
    .collect()
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
        notifications: parse_notification_bell(&c["subscribeButton"]),
      })
    })
    .collect()
}

/// Parses the user's playlists (`FEplaylist_aggregation`, all of them in one response). Mixes
/// (`RD…`) are left out: they have no playlist page.
pub fn parse_playlists(response: &Value) -> Vec<PlaylistSummary> {
  let mut lockups = Vec::new();
  find_all(response, "lockupViewModel", &mut lockups);
  lockups
    .into_iter()
    .filter_map(parse_playlist_lockup)
    .collect()
}

fn parse_playlist_lockup(lockup: &Value) -> Option<PlaylistSummary> {
  if lockup["contentType"] != "LOCKUP_CONTENT_TYPE_PLAYLIST" {
    return None;
  }
  let id = lockup["contentId"]
    .as_str()
    .filter(|id| !id.starts_with("RD"))?;
  let meta = &lockup["metadata"]["lockupMetadataViewModel"];
  let image = &lockup["contentImage"]["collectionThumbnailViewModel"]["primaryThumbnail"]["thumbnailViewModel"];
  let count = image["overlays"]
    .as_array()
    .into_iter()
    .flatten()
    .flat_map(|o| {
      o["thumbnailOverlayBadgeViewModel"]["thumbnailBadges"]
        .as_array()
        .into_iter()
        .flatten()
    })
    .find_map(|b| b["thumbnailBadgeViewModel"]["text"].as_str())
    .map(str::to_owned);
  Some(PlaylistSummary {
    id: id.to_owned(),
    title: meta["title"]["content"].as_str()?.to_owned(),
    count,
    // The last row is a link ("Afficher la playlist complète"), not a detail.
    metadata: meta["metadata"]["contentMetadataViewModel"]["metadataRows"]
      .as_array()
      .into_iter()
      .flatten()
      .flat_map(|row| row["metadataParts"].as_array().into_iter().flatten())
      .filter(|part| part["text"].get("commandRuns").is_none())
      .filter_map(|part| part["text"]["content"].as_str().map(str::to_owned))
      .collect(),
    thumbnails: thumbnails(&image["image"]["sources"]),
  })
}

/// Parses the "Save to playlist" dialog (`playlist/get_add_to_playlist`): every playlist the
/// user can add to, "Watch later" first, and whether it holds the video already.
pub fn parse_playlist_choices(response: &Value) -> Vec<PlaylistChoice> {
  let mut options = Vec::new();
  find_all(response, "playlistAddToOptionRenderer", &mut options);
  options
    .into_iter()
    .filter_map(|o| {
      Some(PlaylistChoice {
        id: o["playlistId"].as_str()?.to_owned(),
        title: non_empty(text(&o["title"]))?,
        // "ALL", "NONE" (and "SOME" when several videos are asked about).
        contains: o["containsSelectedVideos"] == "ALL",
        private: o["privacy"] == "PRIVATE",
      })
    })
    .collect()
}

/// Parses a playlist's page (`VL<id>`): header, first videos and the next videos' token.
pub fn parse_playlist(id: &str, response: &Value) -> Playlist {
  let page = parse_playlist_page(response);
  let mut playlist = Playlist {
    id: id.to_owned(),
    videos: page.items,
    continuation: page.continuation,
    ..Playlist::default()
  };
  let header = &response["header"];
  if let Some(h) = header.get("playlistHeaderRenderer") {
    playlist.title = text(&h["title"]);
    playlist.owner = non_empty(text(&h["ownerText"]));
    playlist.count = non_empty(text(&h["numVideosText"]));
    playlist.thumbnails = thumbnails(
      &h["cinematicContainer"]["cinematicContainerRenderer"]["backgroundImageConfig"]["thumbnail"]
        ["thumbnails"],
    );
  } else {
    let h = &header["pageHeaderRenderer"]["content"]["pageHeaderViewModel"];
    playlist.title = h["title"]["dynamicTextViewModel"]["text"]["content"]
      .as_str()
      .unwrap_or_default()
      .to_owned();
    // The owner is an avatar stack's text ("de TeamFourStar"); the count is the part with a
    // number among "Playlist · 69 vidéos · 494 852 918 vues".
    playlist.owner = h["metadata"]["contentMetadataViewModel"]["metadataRows"]
      .as_array()
      .into_iter()
      .flatten()
      .flat_map(|r| r["metadataParts"].as_array().into_iter().flatten())
      .find_map(|p| p["avatarStack"]["avatarStackViewModel"]["text"]["content"].as_str())
      .map(str::to_owned);
    playlist.count = metadata_rows(&h["metadata"])
      .concat()
      .into_iter()
      .find(|part| part.chars().any(|c| c.is_ascii_digit()));
    playlist.thumbnails =
      thumbnails(&h["heroImage"]["contentPreviewImageViewModel"]["image"]["sources"]);
  }
  if playlist.title.is_empty() {
    playlist.title = id.to_owned();
  }
  playlist
}

/// Videos of a playlist page or of its continuation, with the token of the next ones. The
/// token is taken from the list holding the videos: a playlist page has another continuation
/// item next to that list, which is not the next videos.
pub fn parse_playlist_page(response: &Value) -> Feed {
  let mut page = Feed {
    logged_in: tracking_param(response, "logged_in").as_deref() == Some("1"),
    ..Feed::default()
  };
  for item in find_video_list(response).into_iter().flatten() {
    if let Some(lockup) = item.get("lockupViewModel") {
      page.items.extend(parse_lockup(lockup));
    } else if let Some(renderer) = item.get("playlistVideoRenderer") {
      page.items.extend(parse_playlist_video(renderer));
    } else if item.get("continuationItemViewModel").is_some()
      || item.get("continuationItemRenderer").is_some()
    {
      page.continuation = page.continuation.or_else(|| continuation_token(item));
    }
  }
  page
}

/// The first array with a video in it (a video lockup or a `playlistVideoRenderer`).
fn find_video_list(value: &Value) -> Option<&Vec<Value>> {
  match value {
    Value::Array(items) => {
      let has_video = items.iter().any(|i| {
        i.get("playlistVideoRenderer").is_some()
          || i["lockupViewModel"]["contentType"] == "LOCKUP_CONTENT_TYPE_VIDEO"
      });
      if has_video {
        return Some(items);
      }
      items.iter().find_map(find_video_list)
    }
    Value::Object(map) => map.values().find_map(find_video_list),
    _ => None,
  }
}

/// A "Watch later" entry. `None` for deleted or private videos (`isPlayable: false`).
fn parse_playlist_video(r: &Value) -> Option<VideoItem> {
  if r["isPlayable"] == false {
    return None;
  }
  let byline = &r["shortBylineText"];
  Some(VideoItem {
    id: r["videoId"].as_str()?.to_owned(),
    title: non_empty(text(&r["title"]))?,
    channel: non_empty(text(byline)),
    channel_id: byline["runs"][0]["navigationEndpoint"]["browseEndpoint"]["browseId"]
      .as_str()
      .map(str::to_owned),
    // Runs alternate values and " • " separators.
    metadata: r["videoInfo"]["runs"]
      .as_array()
      .into_iter()
      .flatten()
      .filter_map(|run| run["text"].as_str())
      .map(str::trim)
      .filter(|t| !t.is_empty() && *t != "•")
      .map(str::to_owned)
      .collect(),
    duration: non_empty(text(&r["lengthText"])),
    thumbnails: thumbnails(&r["thumbnail"]["thumbnails"]),
    avatar: Vec::new(),
    set_video_id: removable_entry(r),
  })
}

/// The entry id of a playlist video the user can remove: its menu has an
/// `ACTION_REMOVE_VIDEO` for it (the user's playlists and "Watch later"; "Liked videos" and
/// other channels' playlists have none).
fn removable_entry(renderer: &Value) -> Option<String> {
  let mut endpoints = Vec::new();
  find_all(&renderer["menu"], "playlistEditEndpoint", &mut endpoints);
  endpoints
    .into_iter()
    .flat_map(|e| e["actions"].as_array().into_iter().flatten())
    .find(|a| a["action"] == "ACTION_REMOVE_VIDEO")
    .and_then(|a| a["setVideoId"].as_str())
    .map(str::to_owned)
}

/// The token of a continuation item: `continuationItemRenderer` keeps it in
/// `continuationEndpoint.continuationCommand`, `continuationItemViewModel` in
/// `continuationCommand.innertubeCommand.continuationCommand` (the outer command has none).
fn continuation_token(value: &Value) -> Option<String> {
  match value {
    Value::Object(map) => map
      .get("continuationCommand")
      .and_then(|c| c["token"].as_str())
      .map(str::to_owned)
      .or_else(|| map.values().find_map(continuation_token)),
    Value::Array(values) => values.iter().find_map(continuation_token),
    _ => None,
  }
}

/// Text parts of each row of a `contentMetadataViewModel`.
fn metadata_rows(metadata: &Value) -> Vec<Vec<String>> {
  metadata["contentMetadataViewModel"]["metadataRows"]
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
  let rows = metadata_rows(&meta["metadata"]);
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
    set_video_id: None,
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
