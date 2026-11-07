//! Lenient parsers for InnerTube responses. Shapes verified against `tests/fixtures/`:
//! - grids (home, subscriptions, channel tabs): `richGridRenderer > richItemRenderer >
//!   content > lockupViewModel`, continuation in `continuationItemRenderer`; shelves between the
//!   items in `richSectionRenderer` (subscriptions: "Les plus pertinentes", Shorts);
//! - related videos: video and playlist `lockupViewModel`s under `secondaryResults`;
//! - watch metadata: `videoPrimaryInfoRenderer` and `videoSecondaryInfoRenderer`, the rating
//!   in its `segmentedLikeDislikeButtonViewModel`;
//! - the playlists library: playlist `lockupViewModel`s with a `collectionThumbnailViewModel`;
//!   mixes (`RD…`) are such lockups too, in the home feed (badge "Mix");
//! - a mix's videos (`next` with its `playlistId` alone): `playlistPanelVideoRenderer`s in
//!   `twoColumnWatchNextResults.playlist.playlist`;
//! - a playlist's videos: video `lockupViewModel`s, or `playlistVideoRenderer`s for "Watch
//!   later", next to a `continuationItemViewModel`; its header is a `playlistHeaderRenderer`
//!   ("Watch later", "Liked videos") or a `pageHeaderViewModel` (other playlists);
//! - a channel's subscribe button (`subscribeButtonRenderer` of the watch page's owner and of
//!   `FEchannels` entries): `subscribed`, its `subscribeEndpoint`, its `unsubscribeEndpoint`
//!   inside a confirmation dialog, and the bell, a `subscriptionNotificationToggleButtonRenderer`;
//! - the notifications inbox (`FEnotifications_inbox`): `notificationRenderer`s in
//!   `multiPageMenuNotificationSectionRenderer`s, a next page in `appendContinuationItemsAction`;
//! - search results (`search`): `videoRenderer`s and `channelRenderer`s in an
//!   `itemSectionRenderer`, beside Shorts shelves (`gridShelfViewModel`) and playlist or course
//!   `lockupViewModel`s; a next page in `appendContinuationItemsAction`;
//! - a channel's sort orders: the `chipViewModel`s of its grid's `chipBarViewModel`, each a
//!   `continuationCommand` whose answer replaces the grid (`reloadContinuationItemsCommand`);
//! - a channel's search (its `expandableTabRenderer`): one `videoRenderer` per
//!   `itemSectionRenderer`, a next page in `appendContinuationItemsAction`;
//! - the Shorts feed: a `reelWatchEndpoint` per Short, the first in the `replacementEndpoint`
//!   of `reel/reel_item_watch` beside the `sequenceContinuation`, the next ones in the `entries`
//!   of `reel/reel_watch_sequence` beside a `continuationEndpoint`.

use serde_json::Value;

use std::collections::HashMap;

use crate::models::{
  Caption, Channel, ChannelNotifications, Chapter, ChatAction, ChatBadge, ChatBadgeKind,
  ChatMessage, ChatRun, Comment, CommentPage, Feed, FeedChip, LikeParams, LikeStatus, LinkTarget,
  Live, LiveChatPage, LiveFormat, Notification, NotificationKind, NotificationLevel,
  NotificationPage, NotificationSection, Playlist, PlaylistChoice, PlaylistSummary, Progress,
  Related, SearchPage, SearchResult, ShortsPage, Storyboard, Subscription, TextLink, Thumbnail,
  Tracking, VideoItem, WatchPage,
};

/// Parses a browse response or a browse continuation into a feed.
pub fn parse_feed(response: &Value) -> Feed {
  feed(response, true)
}

/// Parses the subscriptions feed or one of its continuations: a feed without its shelves
/// (`richSectionRenderer`). The "Les plus pertinentes" one comes first and repeats recent videos
/// out of date order, which would hide their place in the list (repeated videos are dropped).
pub fn parse_subscriptions(response: &Value) -> Feed {
  feed(response, false)
}

fn feed(response: &Value, shelves: bool) -> Feed {
  let mut items = Vec::new();
  collect_lockups(response, shelves, &mut items);
  Feed {
    items,
    continuation: find_continuation(response),
    logged_in: tracking_param(response, "logged_in").as_deref() == Some("1"),
    message: find_key(response, "feedNudgeRenderer").map(|nudge| text(&nudge["title"])),
    chips: parse_chips(response),
  }
}

/// The chips of the grid's `chipBarViewModel` (a channel's sort orders), each with the token of
/// its first page; chips without one are skipped.
fn parse_chips(response: &Value) -> Vec<FeedChip> {
  let Some(bar) = find_key(response, "chipBarViewModel") else {
    return Vec::new();
  };
  bar["chips"]
    .as_array()
    .into_iter()
    .flatten()
    .filter_map(|chip| {
      let chip = &chip["chipViewModel"];
      Some(FeedChip {
        label: non_empty(chip["text"].as_str()?.to_owned())?,
        selected: chip["selected"].as_bool().unwrap_or(false),
        token: chip["tapCommand"]["innertubeCommand"]["continuationCommand"]["token"]
          .as_str()?
          .to_owned(),
      })
    })
    .collect()
}

/// Parses a channel's search results (its "Search" tab, or a continuation of it): its videos
/// are `videoRenderer`s, as in the global search.
pub fn parse_channel_search(response: &Value) -> Feed {
  let mut results = Vec::new();
  collect_search_results(response, &mut results);
  Feed {
    items: results
      .into_iter()
      .filter_map(|result| match result {
        // On the channel's own page, as in its grid: no channel name nor picture.
        SearchResult::Video(video) => Some(VideoItem {
          channel: None,
          channel_id: None,
          avatar: Vec::new(),
          ..video
        }),
        SearchResult::Channel(_) | SearchResult::Playlist(_) => None,
      })
      .collect(),
    continuation: find_continuation(response),
    logged_in: tracking_param(response, "logged_in").as_deref() == Some("1"),
    ..Feed::default()
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
  let mut lockups = Vec::new();
  find_all(secondary_results, "lockupViewModel", &mut lockups);
  let related = lockups
    .into_iter()
    .filter_map(|lockup| {
      parse_lockup(lockup)
        .map(Related::Video)
        .or_else(|| parse_playlist_lockup(lockup).map(Related::Playlist))
    })
    .collect();

  let (like_status, like_params) = parse_like_buttons(&primary["videoActions"]);
  let (likes, likes_liked) = parse_like_counts(&primary["videoActions"]);

  // A finished live keeps a chat replay: not a chat to follow.
  let chat = &results["conversationBar"]["liveChatRenderer"];
  let live_chat = (chat["isReplay"] != true)
    .then(|| chat["continuations"][0]["reloadContinuationData"]["continuation"].as_str())
    .flatten()
    .map(str::to_owned);

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
    description_links: attributed_links(&secondary["attributedDescription"]),
    related,
    related_continuation: find_continuation(secondary_results),
    comments_continuation,
    like_status,
    like_params,
    likes,
    likes_liked,
    subscription: parse_subscription(&secondary["subscribeButton"]),
    notifications: parse_notification_bell(&secondary["subscribeButton"]),
    live_chat,
    chapters: parse_chapters(response),
  }
}

/// The chapters of the player bar's markers: the description's (`DESCRIPTION_CHAPTERS`) when
/// there are, else whichever list holds chapters (`AUTO_CHAPTERS`, generated by YouTube).
fn parse_chapters(response: &Value) -> Vec<Chapter> {
  let bar = &response["playerOverlays"]["playerOverlayRenderer"]["decoratedPlayerBarRenderer"]["decoratedPlayerBarRenderer"]
    ["playerBar"]["multiMarkersPlayerBarRenderer"];
  let markers = bar["markersMap"]
    .as_array()
    .map(Vec::as_slice)
    .unwrap_or_default();
  let with_chapters = |m: &&Value| m["value"]["chapters"].is_array();
  let Some(list) = markers
    .iter()
    .filter(with_chapters)
    .find(|m| m["key"] == "DESCRIPTION_CHAPTERS")
    .or_else(|| markers.iter().find(with_chapters))
  else {
    return Vec::new();
  };
  let mut chapters: Vec<Chapter> = list["value"]["chapters"]
    .as_array()
    .map(Vec::as_slice)
    .unwrap_or_default()
    .iter()
    .filter_map(|c| {
      let c = &c["chapterRenderer"];
      Some(Chapter {
        start: c["timeRangeStartMillis"].as_f64()? / 1000.0,
        title: text(&c["title"]),
      })
    })
    .collect();
  chapters.sort_by(|a, b| a.start.total_cmp(&b.start));
  chapters
}

/// Parses a page of a live's chat (`live_chat/get_live_chat`). Text messages and removals are
/// kept; other items (engagement notices, banners, placeholders…) are skipped. The next page's
/// token and delay come from whichever continuation YouTube gives (invalidation, timed or
/// reload).
pub fn parse_live_chat(response: &Value) -> LiveChatPage {
  let chat = &response["continuationContents"]["liveChatContinuation"];
  let next = chat["continuations"][0]
    .as_object()
    .and_then(|c| c.values().find(|v| v["continuation"].is_string()));
  let actions = chat["actions"]
    .as_array()
    .into_iter()
    .flatten()
    .filter_map(|action| {
      if let Some(id) = action["removeChatItemAction"]["targetItemId"].as_str() {
        return Some(ChatAction::Remove(id.to_owned()));
      }
      let item = &action["addChatItemAction"]["item"];
      parse_chat_message(&item["liveChatTextMessageRenderer"]).map(ChatAction::Message)
    })
    .collect();
  LiveChatPage {
    actions,
    continuation: next
      .and_then(|n| n["continuation"].as_str())
      .map(str::to_owned),
    poll_after: std::time::Duration::from_millis(
      next.and_then(|n| n["timeoutMs"].as_u64()).unwrap_or(5000),
    ),
  }
}

/// One `liveChatTextMessageRenderer`; `None` when malformed.
fn parse_chat_message(r: &Value) -> Option<ChatMessage> {
  let id = r["id"].as_str()?.to_owned();
  let runs: Vec<ChatRun> = r["message"]["runs"]
    .as_array()
    .into_iter()
    .flatten()
    .filter_map(|run| {
      if let Some(text) = run["text"].as_str() {
        return Some(ChatRun::Text(text.to_owned()));
      }
      let emoji = &run["emoji"];
      let custom = emoji["isCustomEmoji"] == true;
      let name = if custom {
        emoji["shortcuts"][0].as_str()
      } else {
        emoji["emojiId"].as_str()
      }?;
      Some(ChatRun::Emoji {
        name: name.to_owned(),
        url: thumbnails(&emoji["image"]["thumbnails"])
          .into_iter()
          .max_by_key(|t| t.width)
          .map(|t| t.url),
        custom,
      })
    })
    .collect();
  if runs.is_empty() {
    return None;
  }
  let badges = r["authorBadges"]
    .as_array()
    .into_iter()
    .flatten()
    .filter_map(|b| {
      let badge = &b["liveChatAuthorBadgeRenderer"];
      let image = thumbnails(&badge["customThumbnail"]["thumbnails"])
        .into_iter()
        .max_by_key(|t| t.width)
        .map(|t| t.url);
      let kind = match badge["icon"]["iconType"].as_str() {
        Some("OWNER") => ChatBadgeKind::Owner,
        Some("MODERATOR") => ChatBadgeKind::Moderator,
        Some("VERIFIED") => ChatBadgeKind::Verified,
        _ if image.is_some() => ChatBadgeKind::Member,
        _ => return None,
      };
      Some(ChatBadge {
        kind,
        label: badge["tooltip"].as_str().unwrap_or_default().to_owned(),
        image,
      })
    })
    .collect();
  Some(ChatMessage {
    id,
    author: text(&r["authorName"]),
    author_id: r["authorExternalChannelId"].as_str().map(str::to_owned),
    badges,
    runs,
    timestamp_usec: r["timestampUsec"].as_str().and_then(|t| t.parse().ok()),
  })
}

/// A subscribe button's state and the params of its endpoints.
fn parse_subscription(subscribe_button: &Value) -> Option<Subscription> {
  let button = subscribe_button.get("subscribeButtonRenderer")?;
  let params = |key: &str| {
    find_key(button, key).and_then(|endpoint| endpoint["params"].as_str().map(str::to_owned))
  };
  Some(Subscription {
    subscribed: button["subscribed"] == true,
    subscribe_params: params("subscribeEndpoint"),
    unsubscribe_params: params("unsubscribeEndpoint"),
  })
}

/// The state `subscription/subscribe` or `subscription/unsubscribe` answers for `channel_id`
/// (its `updateSubscribeButtonAction`).
pub fn parse_subscribed(response: &Value, channel_id: &str) -> Option<bool> {
  let mut actions = Vec::new();
  find_all(response, "updateSubscribeButtonAction", &mut actions);
  actions
    .into_iter()
    .find(|a| a["channelId"] == channel_id)
    .and_then(|a| a["subscribed"].as_bool())
}

/// The notification bell of a subscribe button, there even when the user is not subscribed
/// (for after a subscription) or signed out. The current level is the icon of the state
/// `currentStateId` names; the options are the menu items calling
/// `modifyChannelNotificationPreferenceEndpoint` (not "Se désabonner").
fn parse_notification_bell(subscribe_button: &Value) -> Option<ChannelNotifications> {
  let button = &subscribe_button["subscribeButtonRenderer"];
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

/// The like button's counts (its title), untoggled and toggled (liked).
fn parse_like_counts(actions: &Value) -> (Option<String>, Option<String>) {
  let Some(like) = find_key(actions, "likeButtonViewModel") else {
    return Default::default();
  };
  let title = |key| {
    find_key(&like, key)
      .and_then(|b| b["buttonViewModel"]["title"].as_str().map(str::to_owned))
      .filter(|t| !t.is_empty())
  };
  (
    title("defaultButtonViewModel"),
    title("toggledButtonViewModel"),
  )
}

/// Parses the first page of the Shorts feed (`reel/reel_item_watch` without a video): the
/// Short YouTube starts with and the token of the next ones.
pub fn parse_shorts_seed(response: &Value) -> ShortsPage {
  ShortsPage {
    ids: reel_video_id(&response["replacementEndpoint"])
      .into_iter()
      .collect(),
    continuation: response["sequenceContinuation"].as_str().map(str::to_owned),
  }
}

/// Parses a next page of the Shorts feed (`reel/reel_watch_sequence`). Entries without a video
/// are skipped.
pub fn parse_shorts_sequence(response: &Value) -> ShortsPage {
  ShortsPage {
    ids: response["entries"]
      .as_array()
      .into_iter()
      .flatten()
      .filter_map(|e| reel_video_id(&e["command"]))
      .collect(),
    continuation: response["continuationEndpoint"]["continuationCommand"]["token"]
      .as_str()
      .map(str::to_owned),
  }
}

/// Video id of a command holding a `reelWatchEndpoint`.
fn reel_video_id(command: &Value) -> Option<String> {
  command["reelWatchEndpoint"]["videoId"]
    .as_str()
    .filter(|id| !id.is_empty())
    .map(str::to_owned)
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

/// The largest level of a player response's storyboard. The spec is `base|level|level…`,
/// each level `width#height#count#columns#rows#interval_ms#name#sigh`; the sheet URL is the
/// base with `$L` the level's index and `$N` its name (`M$M` for numbered sheets), plus
/// `&sigh=` (format as yt-dlp's `_extract_storyboard` reads it).
pub fn parse_storyboard(player: &Value) -> Option<Storyboard> {
  let spec = player["storyboards"]["playerStoryboardSpecRenderer"]["spec"].as_str()?;
  let mut parts = spec.split('|');
  let base = parts.next()?;
  parts
    .enumerate()
    .filter_map(|(level, args)| {
      let args: Vec<&str> = args.split('#').collect();
      let [width, height, count, columns, rows, interval, name, sigh] = args[..] else {
        return None;
      };
      let number = |s: &str| s.parse::<u32>().ok();
      let board = Storyboard {
        url: format!(
          "{}&sigh={sigh}",
          base.replace("$L", &level.to_string()).replace("$N", name)
        ),
        width: number(width)?,
        height: number(height)?,
        count: number(count)?,
        columns: number(columns)?,
        rows: number(rows)?,
        interval_ms: number(interval)?,
      };
      (board.width > 0 && board.count > 0 && board.columns > 0 && board.rows > 0).then_some(board)
    })
    .max_by_key(|b| b.width)
}

/// Whether a player response is a live happening now (`videoDetails.isLive`), and how far back it
/// can be watched: with `isLiveDvrEnabled`, the `playlist_duration` of its HLS manifest URL (else
/// the window it has now). `length` is what its media playlist covers ([`Live::length`]); a full
/// window measures a little less than `playlist_duration` (3599.9 s for 3600), and that measure
/// wins, so that nothing starts or seeks into segments already gone.
pub fn parse_live(player: &Value, length: f64) -> Option<Live> {
  /// Two of YouTube's live segments.
  const FULL_WINDOW_SLACK: f64 = 10.0;
  let details = &player["videoDetails"];
  if details["isLive"] != true {
    return None;
  }
  let dvr_window = (details["isLiveDvrEnabled"] == true).then(|| {
    let announced = player["streamingData"]["hlsManifestUrl"]
      .as_str()
      .and_then(|url| url.split_once("/playlist_duration/"))
      .and_then(|(_, rest)| rest.split('/').next()?.parse::<f64>().ok())
      .unwrap_or(length);
    if announced - length <= FULL_WINDOW_SLACK {
      length
    } else {
      announced
    }
  });
  Some(Live {
    length,
    dvr_window,
    proxy: None,
  })
}

/// The DASH formats of a live with DVR (`isLive` and `isLiveDvrEnabled`) that can be played
/// through [`crate::LiveProxy`]: a direct URL with no `available_at` delay, MP4 (AVC, AV1, AAC).
/// Not WebM (VP9): FFmpeg plays its segments from the start position but ends the video at the
/// first seek (`video=eof`, the audio going on alone). Empty for anything else, and when the
/// formats lack a video or an audio one.
pub fn parse_live_formats(player: &Value) -> Vec<LiveFormat> {
  let details = &player["videoDetails"];
  if details["isLive"] != true || details["isLiveDvrEnabled"] != true {
    return Vec::new();
  }
  let Some(formats) = player["streamingData"]["adaptiveFormats"].as_array() else {
    return Vec::new();
  };
  let parsed: Vec<LiveFormat> = formats.iter().filter_map(live_format).collect();
  let has = |audio: bool| parsed.iter().any(|f| f.audio() == audio);
  if has(true) && has(false) {
    parsed
  } else {
    Vec::new()
  }
}

fn live_format(format: &Value) -> Option<LiveFormat> {
  let url = format["url"].as_str()?;
  // Such a URL answers 403 until then (ADR 0003).
  if url.contains("available_at") {
    return None;
  }
  let mime_type = format["mimeType"].as_str()?;
  let (mime, codecs) = mime_type.split_once(';')?;
  let codecs = codecs
    .trim()
    .strip_prefix("codecs=")?
    .trim_matches('"')
    .to_owned();
  let playable = match mime {
    "video/mp4" => codecs.starts_with("avc1") || codecs.starts_with("av01"),
    "audio/mp4" => codecs.starts_with("mp4a"),
    _ => false,
  };
  if !playable {
    return None;
  }
  let number = |key: &str| {
    format[key]
      .as_u64()
      .or_else(|| format[key].as_str()?.parse().ok())
  };
  let segment = format["targetDurationSec"].as_f64().filter(|s| *s > 0.0)?;
  Some(LiveFormat {
    itag: u32::try_from(number("itag")?).ok()?,
    url: url.to_owned(),
    mime: mime.to_owned(),
    codecs,
    width: number("width")
      .and_then(|n| u32::try_from(n).ok())
      .unwrap_or(0),
    height: number("height")
      .and_then(|n| u32::try_from(n).ok())
      .unwrap_or(0),
    fps: number("fps").and_then(|n| u32::try_from(n).ok()),
    bitrate: number("bitrate").unwrap_or(0),
    segment,
    max_dvr: format["maxDvrDurationSec"].as_f64().unwrap_or(0.0),
  })
}

/// Parses the subscribed channels list (`FEchannels`: all channels in one response).
pub fn parse_channels(response: &Value) -> Vec<Channel> {
  let mut renderers = Vec::new();
  find_all(response, "channelRenderer", &mut renderers);
  renderers.into_iter().filter_map(parse_channel).collect()
}

/// One `channelRenderer` (the channels list, search results).
fn parse_channel(c: &Value) -> Option<Channel> {
  Some(Channel {
    id: c["channelId"].as_str()?.to_owned(),
    name: text(&c["title"]).trim().to_owned(),
    // YouTube puts the handle in `subscriberCountText` and the subscriber count in
    // `videoCountText`.
    handle: non_empty(text(&c["subscriberCountText"])).filter(|h| h.starts_with('@')),
    subscribers: non_empty(text(&c["videoCountText"])),
    avatar: thumbnails(&c["thumbnail"]["thumbnails"]),
    // A search result's channel carries a bell for after a subscription: not one to show.
    notifications: parse_subscription(&c["subscribeButton"])
      .is_some_and(|s| s.subscribed)
      .then(|| parse_notification_bell(&c["subscribeButton"]))
      .flatten(),
  })
}

/// Parses a page of search results (`search`, or its continuation).
pub fn parse_search(response: &Value) -> SearchPage {
  let mut results = Vec::new();
  collect_search_results(response, &mut results);
  SearchPage {
    results,
    continuation: find_continuation(response),
  }
}

/// Collects the videos, channels and playlists (mixes included) under `value` in document
/// order, skipping malformed ones.
fn collect_search_results(value: &Value, out: &mut Vec<SearchResult>) {
  match value {
    Value::Object(map) => {
      if let Some(video) = map.get("videoRenderer") {
        out.extend(parse_video_renderer(video).map(SearchResult::Video));
      } else if let Some(channel) = map.get("channelRenderer") {
        out.extend(parse_channel(channel).map(SearchResult::Channel));
      } else if let Some(lockup) = map.get("lockupViewModel") {
        out.extend(parse_playlist_lockup(lockup).map(SearchResult::Playlist));
      } else {
        map.values().for_each(|v| collect_search_results(v, out));
      }
    }
    Value::Array(values) => values.iter().for_each(|v| collect_search_results(v, out)),
    _ => {}
  }
}

/// One `videoRenderer` (search results): channel, views · date, duration.
fn parse_video_renderer(video: &Value) -> Option<VideoItem> {
  let id = video["videoId"].as_str()?.to_owned();
  let title = non_empty(text(&video["title"]))?;
  let owner = &video["ownerText"]["runs"][0];
  let views = non_empty(text(&video["shortViewCountText"]))
    .or_else(|| non_empty(text(&video["viewCountText"])));
  Some(VideoItem {
    id,
    title,
    channel: owner["text"].as_str().map(str::to_owned),
    channel_id: owner["navigationEndpoint"]["browseEndpoint"]["browseId"]
      .as_str()
      .map(str::to_owned),
    metadata: views
      .into_iter()
      .chain(non_empty(text(&video["publishedTimeText"])))
      .collect(),
    duration: non_empty(text(&video["lengthText"])),
    live: video["badges"]
      .as_array()
      .into_iter()
      .flatten()
      .any(|b| b["metadataBadgeRenderer"]["style"] == "BADGE_STYLE_TYPE_LIVE_NOW"),
    // Anonymous searches, global or in a channel, left members-only videos out (checked
    // 2026-11-06): no response to parse their badge from.
    members_only: false,
    thumbnails: thumbnails(&video["thumbnail"]["thumbnails"]),
    avatar: thumbnails(
      &video["channelThumbnailSupportedRenderers"]["channelThumbnailWithLinkRenderer"]["thumbnail"]
        ["thumbnails"],
    ),
    set_video_id: None,
    progress: renderer_progress(video),
  })
}

/// Parses a page of the notifications inbox (`FEnotifications_inbox` or its continuation). A
/// next page has no sections: its notifications make one untitled section.
pub fn parse_notifications(response: &Value) -> NotificationPage {
  let mut renderers = Vec::new();
  find_all(
    response,
    "multiPageMenuNotificationSectionRenderer",
    &mut renderers,
  );
  let mut sections = renderers
    .into_iter()
    .map(|section| NotificationSection {
      title: text(&section["notificationSectionTitle"]),
      items: parse_notification_items(&section["items"]),
    })
    .collect::<Vec<_>>();
  if sections.is_empty()
    && let Some(action) = find_key(response, "appendContinuationItemsAction")
  {
    sections.push(NotificationSection {
      title: String::new(),
      items: parse_notification_items(&action["continuationItems"]),
    });
  }
  sections.retain(|s| !s.items.is_empty());
  NotificationPage {
    sections,
    continuation: find_continuation(response),
  }
}

fn parse_notification_items(items: &Value) -> Vec<Notification> {
  items
    .as_array()
    .into_iter()
    .flatten()
    .filter_map(|item| parse_notification(&item["notificationRenderer"]))
    .collect()
}

fn parse_notification(n: &Value) -> Option<Notification> {
  let endpoint = &n["navigationEndpoint"];
  let reel = endpoint["reelWatchEndpoint"]["videoId"].as_str();
  let video_id = endpoint["watchEndpoint"]["videoId"].as_str().or(reel);
  Some(Notification {
    id: n["notificationId"].as_str()?.to_owned(),
    // The channel's run ends with spaces (`Asmongold TV   a mis en ligne`).
    message: text(&n["shortMessage"])
      .split_whitespace()
      .collect::<Vec<_>>()
      .join(" "),
    sent: text(&n["sentTimeText"]),
    read: n["read"].as_bool().unwrap_or(true),
    video_id: video_id.map(str::to_owned),
    short: reel.is_some(),
    kind: notification_kind(endpoint["clickTrackingParams"].as_str().unwrap_or_default()),
    avatar: thumbnails(&n["thumbnail"]["thumbnails"]),
    thumbnails: thumbnails(&n["videoThumbnail"]["thumbnails"]),
    read_token: n["recordClickEndpoint"]["recordNotificationInteractionsEndpoint"]
      ["serializedInteractionsRequest"]
      .as_str()
      .map(str::to_owned),
  })
}

/// What a notification announces, from its click's tracking params: base64 bytes holding a
/// label, `ni-push-u-sub` for an upload (a Short included), `ni-push-lbss` for a live
/// (checked 2026-11-03 against the inbox in French; the label is not translated).
fn notification_kind(tracking: &str) -> NotificationKind {
  let bytes = base64_decode(tracking);
  let has = |label: &[u8]| bytes.windows(label.len()).any(|w| w == label);
  if has(b"ni-push-lbss") {
    NotificationKind::Live
  } else if has(b"ni-push-u-sub") {
    NotificationKind::Upload
  } else {
    NotificationKind::Other
  }
}

/// Decodes base64, standard or URL-safe, padded or not; stops at the first byte outside it.
fn base64_decode(text: &str) -> Vec<u8> {
  let value = |c: u8| match c {
    b'A'..=b'Z' => Some(c - b'A'),
    b'a'..=b'z' => Some(c - b'a' + 26),
    b'0'..=b'9' => Some(c - b'0' + 52),
    b'+' | b'-' => Some(62),
    b'/' | b'_' => Some(63),
    _ => None,
  };
  let mut out = Vec::with_capacity(text.len() * 3 / 4);
  let (mut buffer, mut bits) = (0u32, 0u32);
  for sextet in text.bytes().map_while(value) {
    buffer = (buffer << 6) | u32::from(sextet);
    bits += 6;
    if bits >= 8 {
      bits -= 8;
      // The byte is the 8 bits above the `bits` left over.
      out.push((buffer >> bits) as u8);
      buffer &= (1 << bits) - 1;
    }
  }
  out
}

/// The title of an oEmbed response (`/oembed?url=…`): the video's own, never translated into
/// the request's language as InnerTube's titles are.
pub fn parse_oembed_title(response: &Value) -> Option<String> {
  response["title"]
    .as_str()
    .filter(|title| !title.is_empty())
    .map(str::to_owned)
}

/// The unseen count of a `notification/get_unseen_count` response.
pub fn parse_unseen_count(response: &Value) -> Option<u32> {
  find_key(response, "updateNotificationsUnseenCountAction")?["unseenCount"]
    .as_u64()
    .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
}

/// Parses the user's playlists (`FEplaylist_aggregation`, all of them in one response), saved
/// mixes included.
pub fn parse_playlists(response: &Value) -> Vec<PlaylistSummary> {
  let mut lockups = Vec::new();
  find_all(response, "lockupViewModel", &mut lockups);
  lockups
    .into_iter()
    .filter_map(parse_playlist_lockup)
    .collect()
}

/// Whether `id` is a mix's: a playlist YouTube makes and keeps endless (`RD…`, "My Mix" is
/// `RDMM`). It has no playlist page (`VL…`): [`parse_mix`] reads it from `next`.
pub fn is_mix(id: &str) -> bool {
  id.starts_with("RD")
}

/// The mixes among the cards of a response (a page of the home feed, the playlists library),
/// in order.
pub fn parse_mixes(response: &Value) -> Vec<PlaylistSummary> {
  let mut lockups = Vec::new();
  find_all(response, "lockupViewModel", &mut lockups);
  lockups
    .into_iter()
    .filter_map(parse_playlist_lockup)
    .filter(|p| is_mix(&p.id))
    .collect()
}

/// Parses a mix (`next` with its `playlistId` alone): its title and the videos of its panel
/// (`playlistPanelVideoRenderer`s, about 25). A mix is endless: YouTube adds to it as it
/// plays, so it has no next page here.
pub fn parse_mix(id: &str, response: &Value) -> Playlist {
  let panel = &response["contents"]["twoColumnWatchNextResults"]["playlist"]["playlist"];
  let videos: Vec<VideoItem> = panel["contents"]
    .as_array()
    .into_iter()
    .flatten()
    .filter_map(|c| parse_playlist_video(&c["playlistPanelVideoRenderer"]))
    // Its menu edits the queue being played on youtube.com, not a playlist.
    .map(|v| VideoItem {
      set_video_id: None,
      ..v
    })
    .collect();
  Playlist {
    id: id.to_owned(),
    title: panel["title"]
      .as_str()
      .filter(|t| !t.is_empty())
      .unwrap_or(id)
      .to_owned(),
    owner: non_empty(text(&panel["ownerName"])),
    count: None,
    thumbnails: videos
      .first()
      .map(|v| v.thumbnails.clone())
      .unwrap_or_default(),
    videos,
    continuation: None,
  }
}

/// One playlist lockup (a mix too): of the library, the home feed, a search (where courses are
/// playlists too) or the related videos. Its rows hold plain details ("Privée", "Playlist"),
/// the byline's channel link, links to its first videos and to its page, and a description:
/// only the first two are kept.
fn parse_playlist_lockup(lockup: &Value) -> Option<PlaylistSummary> {
  if !matches!(
    lockup["contentType"].as_str(),
    Some("LOCKUP_CONTENT_TYPE_PLAYLIST" | "LOCKUP_CONTENT_TYPE_COURSE")
  ) {
    return None;
  }
  let id = lockup["contentId"].as_str()?;
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
  let mut channel = None;
  let mut metadata = Vec::new();
  let rows = meta["metadata"]["contentMetadataViewModel"]["metadataRows"]
    .as_array()
    .into_iter()
    .flatten()
    .filter(|row| {
      row["lockupContentMetadataRowExtension"]["contentType"]
        != "METADATA_ROW_CONTENT_TYPE_DESCRIPTION"
    });
  for part in rows.flat_map(|row| row["metadataParts"].as_array().into_iter().flatten()) {
    let Some(content) = part["text"]["content"].as_str() else {
      continue;
    };
    let Some(runs) = part["text"].get("commandRuns") else {
      metadata.push(content.to_owned());
      continue;
    };
    // Its channel's name, then a label leading there too ("Playlist", "Cours"); the links to
    // its videos and to its page are no details.
    let browse = runs[0]["onTap"]["innertubeCommand"]["browseEndpoint"]["browseId"].as_str();
    match browse.filter(|id| id.starts_with("UC")) {
      Some(channel_id) if channel.is_none() => {
        channel = Some((content.to_owned(), channel_id.to_owned()));
      }
      Some(_) => metadata.push(content.to_owned()),
      None => {}
    }
  }
  let (channel, channel_id) = channel.unzip();
  Some(PlaylistSummary {
    id: id.to_owned(),
    title: meta["title"]["content"].as_str()?.to_owned(),
    count,
    channel,
    channel_id,
    metadata,
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
    live: false,
    members_only: false,
    thumbnails: thumbnails(&r["thumbnail"]["thumbnails"]),
    avatar: Vec::new(),
    set_video_id: removable_entry(r),
    progress: renderer_progress(r),
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
        .collect::<Vec<_>>()
    })
    // A row of badges only ("Members only") has no text: it is not the channel's row.
    .filter(|row| !row.is_empty())
    .collect()
}

/// Whether a lockup's metadata carries the "Members only" badge (a row of `badgeViewModel`s).
fn lockup_members_only(metadata: &Value) -> bool {
  metadata["contentMetadataViewModel"]["metadataRows"]
    .as_array()
    .into_iter()
    .flatten()
    .flat_map(|row| row["badges"].as_array().into_iter().flatten())
    .any(|b| b["badgeViewModel"]["badgeStyle"] == "BADGE_MEMBERS_ONLY")
}

/// Whether a WEB `player` answer offers the channel's membership instead of the video: a
/// members-only video the session cannot watch (`offerId` is not translated, unlike `reason`).
pub fn parse_members_only(player: &Value) -> bool {
  player["playabilityStatus"]["status"] != "OK"
    && player["playabilityStatus"]["errorScreen"]["playerLegacyDesktopYpcOfferRenderer"]["offerId"]
      == "sponsors_only_video"
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
    links: attributed_links(&properties["content"]),
    published: string(&properties["publishedTime"]),
    likes: string(&toolbar["likeCountNotliked"]),
    replies: string(&toolbar["replyCount"]),
    replies_continuation: None,
  })
}

/// The links of an attributed string (`{content, commandRuns}`). Runs count UTF-16 code units:
/// a run that does not fall on characters of the text, or overlaps the previous one, is skipped.
fn attributed_links(attributed: &Value) -> Vec<TextLink> {
  let content = attributed["content"].as_str().unwrap_or_default();
  // Byte offset of each UTF-16 offset that starts a character, the text's end included.
  let mut bytes = HashMap::new();
  let mut utf16 = 0;
  for (byte, c) in content.char_indices() {
    bytes.insert(utf16, byte);
    utf16 += c.len_utf16();
  }
  bytes.insert(utf16, content.len());
  let mut links: Vec<TextLink> = Vec::new();
  for run in attributed["commandRuns"].as_array().into_iter().flatten() {
    let (Some(start), Some(length)) = (run["startIndex"].as_u64(), run["length"].as_u64()) else {
      continue;
    };
    let (Some(&start), Some(&end)) = (
      bytes.get(&(start as usize)),
      bytes.get(&((start + length) as usize)),
    ) else {
      continue;
    };
    if start >= end || links.last().is_some_and(|l| l.range.end > start) {
      continue;
    }
    let label = &content[start..end];
    if let Some(target) = link_target(&run["onTap"]["innertubeCommand"], label) {
      links.push(TextLink {
        range: start..end,
        target,
      });
    }
  }
  links
}

/// Where a run's command leads. `label` is the run's text, which names channels and playlists:
/// their chips pad it with no-break spaces, an icon and a `/`.
fn link_target(command: &Value, label: &str) -> Option<LinkTarget> {
  let label = || {
    label
      .trim_matches(|c: char| !c.is_alphanumeric() && c != '@')
      .to_owned()
  };
  if let Some(watch) = command.get("watchEndpoint") {
    return Some(LinkTarget::Video {
      id: watch["videoId"].as_str()?.to_owned(),
      start: watch["startTimeSeconds"]
        .as_u64()
        .and_then(|s| u32::try_from(s).ok()),
    });
  }
  if let Some(url) = command["urlEndpoint"]["url"].as_str() {
    return web(unredirect(url)).map(LinkTarget::Url);
  }
  let browse = &command["browseEndpoint"];
  if let Some(id) = browse["browseId"].as_str() {
    if id.starts_with("UC") {
      return Some(LinkTarget::Channel {
        id: id.to_owned(),
        name: label(),
      });
    }
    if let Some(playlist) = id.strip_prefix("VL") {
      return Some(LinkTarget::Playlist {
        id: playlist.to_owned(),
        title: label(),
      });
    }
  }
  let path = command["commandMetadata"]["webCommandMetadata"]["url"].as_str()?;
  let url = if path.starts_with('/') {
    format!("https://www.youtube.com{path}")
  } else {
    path.to_owned()
  };
  web(url).map(LinkTarget::Page)
}

/// `url` when it is a web page: Windows would hand any other scheme (`ms-msdt:`, `search-ms:`…)
/// to its handler, from text any commenter writes.
fn web(url: String) -> Option<String> {
  reqwest::Url::parse(&url)
    .ok()
    .filter(|u| matches!(u.scheme(), "http" | "https"))
    .map(|_| url)
}

/// The page behind YouTube's redirect (`youtube.com/redirect?…&q=<url>`), else `url` itself.
fn unredirect(url: &str) -> String {
  reqwest::Url::parse(url)
    .ok()
    .filter(|u| u.host_str() == Some("www.youtube.com") && u.path() == "/redirect")
    .and_then(|u| {
      u.query_pairs()
        .find(|(k, _)| k == "q")
        .map(|(_, v)| v.into_owned())
    })
    .unwrap_or_else(|| url.to_owned())
}

/// Collects every video lockup under `value`, skipping malformed ones, and those of the grid's
/// shelves (`richSectionRenderer`) unless `shelves`.
fn collect_lockups(value: &Value, shelves: bool, out: &mut Vec<VideoItem>) {
  match value {
    Value::Object(map) => {
      if let Some(lockup) = map.get("lockupViewModel") {
        if let Some(item) = parse_lockup(lockup) {
          out.push(item);
        }
        return;
      }
      if !shelves && map.contains_key("richSectionRenderer") {
        return;
      }
      map.values().for_each(|v| collect_lockups(v, shelves, out));
    }
    Value::Array(values) => values.iter().for_each(|v| collect_lockups(v, shelves, out)),
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
  let badges: Vec<&Value> = image["overlays"]
    .as_array()
    .into_iter()
    .flatten()
    .flat_map(|o| {
      o["thumbnailBottomOverlayViewModel"]["badges"]
        .as_array()
        .into_iter()
        .flatten()
    })
    .map(|b| &b["thumbnailBadgeViewModel"])
    .collect();
  let duration = badges
    .iter()
    .find_map(|b| b["text"].as_str())
    .filter(|t| t.contains(':'))
    .map(str::to_owned);
  let live = badges
    .iter()
    .any(|b| b["badgeStyle"] == "THUMBNAIL_OVERLAY_BADGE_STYLE_LIVE");

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
    live,
    members_only: lockup_members_only(&meta["metadata"]),
    thumbnails: thumbnails(&image["image"]["sources"]),
    avatar: thumbnails(&avatar["avatar"]["avatarViewModel"]["image"]["sources"]),
    set_video_id: None,
    progress: lockup_progress(lockup),
  })
}

/// The red bar of a video lockup (`thumbnailOverlayProgressBarViewModel`) and where its click
/// resumes.
fn lockup_progress(lockup: &Value) -> Option<Progress> {
  let percent = lockup["contentImage"]["thumbnailViewModel"]["overlays"]
    .as_array()?
    .iter()
    .find_map(|o| {
      o["thumbnailBottomOverlayViewModel"]["progressBar"]["thumbnailOverlayProgressBarViewModel"]
        ["startPercent"]
        .as_u64()
    })?;
  let endpoint = &lockup["rendererContext"]["commandContext"]["onTap"]["innertubeCommand"];
  progress(percent, &endpoint["watchEndpoint"])
}

/// The same for a `videoRenderer` or a `playlistVideoRenderer`
/// (`thumbnailOverlayResumePlaybackRenderer`).
fn renderer_progress(renderer: &Value) -> Option<Progress> {
  let percent = renderer["thumbnailOverlays"]
    .as_array()?
    .iter()
    .find_map(|o| o["thumbnailOverlayResumePlaybackRenderer"]["percentDurationWatched"].as_u64())?;
  progress(percent, &renderer["navigationEndpoint"]["watchEndpoint"])
}

fn progress(percent: u64, watch_endpoint: &Value) -> Option<Progress> {
  let percent = u8::try_from(percent.min(100)).ok().filter(|&p| p > 0)?;
  Some(Progress {
    percent,
    resume: watch_endpoint["startTimeSeconds"]
      .as_u64()
      .and_then(|s| u32::try_from(s).ok())
      .filter(|&s| s > 0),
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
pub(crate) fn find_continuation(value: &Value) -> Option<String> {
  let item = find_key(value, "continuationItemRenderer")?;
  find_key(&item, "continuationCommand")?["token"]
    .as_str()
    .map(str::to_owned)
}

/// The stats URLs of a WEB `player` response, asked with `client_version`; `None` when it has
/// none (a signed-out or unplayable response may lack them).
pub fn parse_tracking(player: &Value, client_version: &str) -> Option<Tracking> {
  let url = |key: &str| {
    player["playbackTracking"][key]["baseUrl"]
      .as_str()
      .filter(|u| u.starts_with("https://"))
      .map(str::to_owned)
  };
  Some(Tracking {
    playback: url("videostatsPlaybackUrl")?,
    watchtime: url("videostatsWatchtimeUrl")?,
    cpn: crate::client::new_cpn(),
    client_version: client_version.to_owned(),
  })
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

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn base64_both_alphabets() {
    assert_eq!(base64_decode("Zm9vYmFy"), b"foobar");
    assert_eq!(base64_decode("Zm9vYg=="), b"foob");
    assert_eq!(base64_decode("Zm9vYg"), b"foob");
    assert_eq!(base64_decode("-_8"), [0xfb, 0xff]);
    assert_eq!(base64_decode("+/8="), [0xfb, 0xff]);
  }

  #[test]
  fn notification_kinds_from_their_label() {
    // `ni-push-lbss` / `ni-push-u-sub` inside the tracking params' bytes.
    let live = "CDAQmlAYDCITCJfTl6md7JcDFTg4BgAd4NEVTTIMbmktcHVzaC1sYnNzygEEj1qcMQ==";
    let upload = "CH4QmlAYACITCJfTl6md7JcDFTg4BgAd4NEVTTINbmktcHVzaC11LXN1YsoBBI9anDE=";
    assert_eq!(notification_kind(live), NotificationKind::Live);
    assert_eq!(notification_kind(upload), NotificationKind::Upload);
    assert_eq!(notification_kind(""), NotificationKind::Other);
  }

  #[test]
  fn links_open_web_pages_only() {
    let redirect = |q: &str| {
      serde_json::json!({ "urlEndpoint": {
        "url": format!("https://www.youtube.com/redirect?event=comments&q={q}")
      }})
    };
    assert_eq!(
      link_target(&redirect("https%3A%2F%2Fexample.com%2Fa%3Fb%3D1"), ""),
      Some(LinkTarget::Url("https://example.com/a?b=1".into()))
    );
    assert_eq!(link_target(&redirect("ms-msdt%3A%2Fid"), ""), None);
    let page = serde_json::json!({ "commandMetadata": { "webCommandMetadata": {
      "url": "search-ms:query=x"
    }}});
    assert_eq!(link_target(&page, ""), None);
  }
}
