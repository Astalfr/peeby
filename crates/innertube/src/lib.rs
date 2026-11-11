//! Client for YouTube's private InnerTube API (the JSON API behind www.youtube.com).
//!
//! There is no official API for the home feed, so this crate speaks InnerTube directly:
//! - [`Client::home`]: the "Home" recommendations (`FEwhat_to_watch`), personalized only when
//!   signed in with browser [`Cookies`] (anonymous Home is empty since 2024);
//! - [`Client::subscriptions`] (newest first, without YouTube's shelves),
//!   [`Client::subscribed_channels`], [`Client::channel_videos`] (sorted by its
//!   [`Feed::chips`]), [`Client::channel_search`], [`Client::channel_search_more`];
//! - [`Client::search`], [`Client::search_more`]: videos and channels matching a query;
//! - [`Client::watch`]: watch page metadata, related videos and the comments token (`next`);
//! - [`Client::shorts`], [`Client::shorts_more`]: the Shorts feed, an endless sequence picked
//!   by YouTube (`reel/…`), each Short's metadata then from [`Client::watch`];
//! - [`Client::comments`]: comment pages and replies (`next` continuations);
//!   [`Client::rate_comment`] rates one, [`Client::translate_comment`] translates one;
//! - [`Client::rate`]: likes, dislikes or clears the user's rating of a video (`like/…`);
//! - [`Client::set_subscribed`]: subscribes to a channel or unsubscribes
//!   (`subscription/subscribe`, `subscription/unsubscribe`), its state read from the watch page;
//! - [`Client::set_channel_notifications`]: the bell of a subscribed channel
//!   (`notification/modify_channel_preference`), its state read from the watch page and the
//!   channels list;
//! - [`Client::notifications`], [`Client::notifications_more`], [`Client::unseen_notifications`]
//!   and [`Client::mark_notification_read`]: the notifications inbox (`FEnotifications_inbox`);
//! - [`Client::playlists`], [`Client::playlist`], [`Client::playlist_more`]: the user's
//!   playlists and their videos; [`Client::playlist_choices`], [`Client::add_to_playlist`] and
//!   [`Client::remove_from_playlist`] save a video to one of them;
//! - [`Client::tracking`], [`Client::report_playback`], [`Client::report_watchtime`]: the
//!   account's watch history and resume positions (the stats pings youtube.com sends, ADR 0013);
//!   the cards' [`Progress`] reads them back;
//! - [`Client::votes`]: likes and dislikes from Return YouTube Dislike, another service:
//!   attribution required, at most 100 requests/min and 10,000/day, so cache the results;
//! - [`Client::stream`]: playable HLS streams through the VISIONOS client, which needs no JS
//!   challenge and has no `available_at` delay (see docs/decisions/0003-visionos-hls-streams.md).
//!
//! Responses are parsed leniently from `serde_json::Value`: YouTube changes renderers often,
//! and one malformed item must never fail a whole feed. Every parser is tested against real
//! responses in `tests/fixtures/`.

mod auth;
mod client;
mod error;
pub mod hls;
mod live;
mod models;
mod parse;
mod proto;

pub use auth::{Cookies, parse_rotation_delay};
pub use client::{Client, SaveCookies, VISIONOS_USER_AGENT, hls_manifest_url, save_to_file};
pub use error::Error;
pub use live::LiveProxy;
pub use models::{
  Caption, Channel, ChannelHeader, ChannelNotifications, ChannelTab, ChannelTabKind, Chapter,
  ChatAction, ChatBadge, ChatBadgeKind, ChatMessage, ChatRun, Collaborator, Comment,
  CommentActions, CommentPage, Feed, FeedChip, LikeParams, LikeStatus, LinkTarget, Live,
  LiveChatPage, LiveFormat, NotInterested, Notification, NotificationKind, NotificationLevel,
  NotificationPage, NotificationSection, Playlist, PlaylistChoice, PlaylistSummary, Progress,
  Related, SearchPage, SearchResult, SearchSuggestion, ShortItem, ShortsPage, SponsorCategory,
  SponsorSegment, Storyboard, StoryboardFrame, Stream, Subscription, SuggestedChannel, TextLink,
  Thumbnail, Tracking, VideoItem, Votes, WatchPage, Watchtime,
};
pub use parse::{
  is_mix, parse_account_handle, parse_captions, parse_channel_search, parse_channels,
  parse_comment_action, parse_comment_translation, parse_comments, parse_feed, parse_feedback,
  parse_live, parse_live_chat, parse_live_formats, parse_members_only, parse_mix, parse_mixes,
  parse_notifications, parse_oembed_title, parse_playlist, parse_playlist_choices,
  parse_playlist_page, parse_playlists, parse_search, parse_search_suggestions, parse_shorts_seed,
  parse_shorts_sequence, parse_sponsor_segments, parse_storyboard, parse_subscribed,
  parse_subscriptions, parse_tracking, parse_unseen_count, parse_watch,
};
