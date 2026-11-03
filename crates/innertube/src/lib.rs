//! Client for YouTube's private InnerTube API (the JSON API behind www.youtube.com).
//!
//! There is no official API for the home feed, so this crate speaks InnerTube directly:
//! - [`Client::home`]: the "Home" recommendations (`FEwhat_to_watch`), personalized only when
//!   signed in with browser [`Cookies`] (anonymous Home is empty since 2024);
//! - [`Client::subscriptions`] (newest first, without YouTube's shelves),
//!   [`Client::subscribed_channels`], [`Client::channel_videos`];
//! - [`Client::search`], [`Client::search_more`]: videos and channels matching a query;
//! - [`Client::watch`]: watch page metadata, related videos and the comments token (`next`);
//! - [`Client::shorts`], [`Client::shorts_more`]: the Shorts feed, an endless sequence picked
//!   by YouTube (`reel/…`), each Short's metadata then from [`Client::watch`];
//! - [`Client::comments`]: comment pages and replies (`next` continuations);
//! - [`Client::rate`]: likes, dislikes or clears the user's rating of a video (`like/…`);
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
mod models;
mod parse;

pub use auth::Cookies;
pub use client::{Client, VISIONOS_USER_AGENT, hls_manifest_url};
pub use error::Error;
pub use models::{
  Caption, Channel, ChannelNotifications, Chapter, ChatAction, ChatBadge, ChatBadgeKind,
  ChatMessage, ChatRun, Comment, CommentPage, Feed, LikeParams, LikeStatus, LinkTarget, Live,
  LiveChatPage, Notification, NotificationLevel, NotificationPage, NotificationSection, Playlist,
  PlaylistChoice, PlaylistSummary, Progress, SearchPage, SearchResult, ShortsPage, Storyboard,
  StoryboardFrame, Stream, TextLink, Thumbnail, Tracking, VideoItem, Votes, WatchPage, Watchtime,
};
pub use parse::{
  parse_captions, parse_channels, parse_comments, parse_feed, parse_live, parse_live_chat,
  parse_notifications, parse_playlist, parse_playlist_choices, parse_playlist_page,
  parse_playlists, parse_search, parse_shorts_seed, parse_shorts_sequence, parse_storyboard,
  parse_subscriptions, parse_tracking, parse_unseen_count, parse_watch,
};
