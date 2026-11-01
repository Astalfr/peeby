/// An image in several sizes, as YouTube lists them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thumbnail {
  /// Image URL (i.ytimg.com, yt3.ggpht.com).
  pub url: String,
  /// Width in pixels, 0 when unknown.
  pub width: u32,
  /// Height in pixels, 0 when unknown.
  pub height: u32,
}

impl Thumbnail {
  /// The smallest of `sizes` at least `width` pixels wide, else the largest.
  pub fn pick(sizes: &[Thumbnail], width: u32) -> Option<&Thumbnail> {
    sizes
      .iter()
      .filter(|t| t.width >= width)
      .min_by_key(|t| t.width)
      .or_else(|| sizes.iter().max_by_key(|t| t.width))
  }
}

/// A video card (home, subscriptions, channel tab, related videos).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoItem {
  /// 11-character video id.
  pub id: String,
  /// Title.
  pub title: String,
  /// Channel name. `None` on a channel's own tab, where the channel is implicit.
  pub channel: Option<String>,
  /// Channel id (`UC…`).
  pub channel_id: Option<String>,
  /// Localized metadata as YouTube formats it, e.g. `["1,2 M vues", "il y a 3 ans"]`.
  pub metadata: Vec<String>,
  /// Duration badge, e.g. `3:51`; `None` for lives and premieres.
  pub duration: Option<String>,
  /// Thumbnail sizes.
  pub thumbnails: Vec<Thumbnail>,
  /// Channel avatar sizes.
  pub avatar: Vec<Thumbnail>,
  /// In a playlist the user can edit: the id of this entry, for
  /// [`crate::Client::remove_playlist_entry`] (a video may be in a playlist twice).
  pub set_video_id: Option<String>,
}

/// A page of videos and the token of the next page.
#[derive(Clone, Debug, Default)]
pub struct Feed {
  /// Videos in display order.
  pub items: Vec<VideoItem>,
  /// Token for [`crate::Client::continuation`], when there is more.
  pub continuation: Option<String>,
  /// Whether YouTube served the request as signed in (`logged_in=1`).
  pub logged_in: bool,
  /// Message YouTube shows instead of videos (e.g. "Your YouTube history is off").
  pub message: Option<String>,
}

/// Metadata of the watch page.
#[derive(Clone, Debug, Default)]
pub struct WatchPage {
  /// Video id.
  pub id: String,
  /// Title.
  pub title: String,
  /// Channel name.
  pub channel: Option<String>,
  /// Channel id (`UC…`).
  pub channel_id: Option<String>,
  /// Channel avatar sizes.
  pub channel_avatar: Vec<Thumbnail>,
  /// Localized subscriber count, e.g. `4,55 M d'abonnés`.
  pub subscribers: Option<String>,
  /// Localized view count.
  pub views: Option<String>,
  /// Localized publication date.
  pub date: Option<String>,
  /// Description, plain text.
  pub description: String,
  /// Related videos.
  pub related: Vec<VideoItem>,
  /// Token of the next page of related videos.
  pub related_continuation: Option<String>,
  /// Token of the first comments page (`next` continuation).
  pub comments_continuation: Option<String>,
  /// The user's rating of the video (`Indifferent` signed out).
  pub like_status: LikeStatus,
  /// What the page's like and dislike buttons send.
  pub like_params: LikeParams,
  /// The channel's notification bell, when the user is subscribed to it.
  pub notifications: Option<ChannelNotifications>,
}

/// A user's rating of a video, as `likeStatusEntity` gives it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LikeStatus {
  /// Neither liked nor disliked.
  #[default]
  Indifferent,
  /// Liked ("J'aime").
  Like,
  /// Disliked ("Je n'aime pas").
  Dislike,
}

/// The `params` of the watch page's `likeEndpoint`s, one per status they set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LikeParams {
  /// `likeParams`, for [`LikeStatus::Like`].
  pub like: Option<String>,
  /// `dislikeParams`, for [`LikeStatus::Dislike`].
  pub dislike: Option<String>,
  /// `removeLikeParams`, for [`LikeStatus::Indifferent`].
  pub remove: Option<String>,
}

impl LikeParams {
  /// The params of the endpoint setting `status`.
  pub fn get(&self, status: LikeStatus) -> Option<&str> {
    match status {
      LikeStatus::Like => self.like.as_deref(),
      LikeStatus::Dislike => self.dislike.as_deref(),
      LikeStatus::Indifferent => self.remove.as_deref(),
    }
  }
}

/// Preview thumbnails over the seek bar: frames of the video laid out in a grid on sprite
/// sheets (`storyboards.playerStoryboardSpecRenderer` of the player response, its largest
/// level).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Storyboard {
  /// Sheet URL with `$M` in place of the sheet index.
  pub url: String,
  /// Width of one frame in pixels.
  pub width: u32,
  /// Height of one frame in pixels.
  pub height: u32,
  /// Frames in the whole storyboard.
  pub count: u32,
  /// Frames per row of a sheet.
  pub columns: u32,
  /// Rows per sheet.
  pub rows: u32,
  /// Time between two frames, 0 when the frames are spread over the whole video.
  pub interval_ms: u32,
}

/// One frame of a storyboard: its sheet and where it sits on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoryboardFrame {
  /// Sheet URL (i.ytimg.com).
  pub url: String,
  /// Column of the frame on the sheet, from 0.
  pub column: u32,
  /// Row of the frame on the sheet, from 0.
  pub row: u32,
}

impl Storyboard {
  /// The frame shown at `seconds` into a video of `duration` seconds.
  pub fn frame(&self, seconds: f64, duration: f64) -> Option<StoryboardFrame> {
    let per_sheet = self.columns * self.rows;
    if self.count == 0 || per_sheet == 0 {
      return None;
    }
    let interval = if self.interval_ms > 0 {
      f64::from(self.interval_ms) / 1000.
    } else if duration > 0. {
      duration / f64::from(self.count)
    } else {
      return None;
    };
    let index = ((seconds.max(0.) / interval) as u32).min(self.count - 1);
    let (sheet, cell) = (index / per_sheet, index % per_sheet);
    Some(StoryboardFrame {
      url: self.url.replace("$M", &sheet.to_string()),
      column: cell % self.columns,
      row: cell / self.columns,
    })
  }
}

/// A subtitle track (WebVTT), fetched by mpv when the user picks it.
#[derive(Clone, PartialEq, Eq)]
pub struct Caption {
  /// Language code, e.g. `en`, `pt-BR`.
  pub language: String,
  /// Name as YouTube shows it, in the client's language.
  pub name: String,
  /// Generated by speech recognition (`asr`).
  pub auto_generated: bool,
  /// WebVTT URL. Contains the user's IP: never log it.
  pub url: String,
}

impl std::fmt::Debug for Caption {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "Caption({} {})", self.language, self.name)
  }
}

/// A playable stream: an HLS video variant and its separate audio rendition.
#[derive(Clone, PartialEq, Eq)]
pub struct Stream {
  /// Video media playlist URL. Contains the user's IP: never log it.
  pub video_url: String,
  /// Audio media playlist URL (YouTube's HLS variants are video only).
  pub audio_url: Option<String>,
  /// `RESOLUTION` of the variant, e.g. `1920x1080`.
  pub resolution: String,
  /// `CODECS` of the variant.
  pub codecs: String,
  /// User-Agent the playlists and segments must be fetched with.
  pub user_agent: &'static str,
  /// Every quality of the master playlist, highest first (the chosen one included).
  pub qualities: Vec<crate::hls::Variant>,
  /// Subtitle tracks.
  pub captions: Vec<Caption>,
  /// Preview thumbnails for the seek bar, when YouTube has them (not for lives).
  pub storyboard: Option<Storyboard>,
}

impl std::fmt::Debug for Stream {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    // The URLs embed the user's IP and signatures.
    f.debug_struct("Stream")
      .field("resolution", &self.resolution)
      .field("codecs", &self.codecs)
      .field("audio", &self.audio_url.is_some())
      .finish_non_exhaustive()
  }
}

/// One search result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchResult {
  /// A video.
  Video(VideoItem),
  /// A channel.
  Channel(Channel),
}

/// A page of search results and the token of the next page.
#[derive(Clone, Debug, Default)]
pub struct SearchPage {
  /// Videos and channels in display order; Shorts shelves, playlists and courses are left out.
  pub results: Vec<SearchResult>,
  /// Token for [`crate::Client::search_more`], when there is more.
  pub continuation: Option<String>,
}

/// A channel the user is subscribed to, or found by a search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
  /// Channel id (`UC…`).
  pub id: String,
  /// Display name.
  pub name: String,
  /// Handle, e.g. `@AsmonTV`.
  pub handle: Option<String>,
  /// Localized subscriber count.
  pub subscribers: Option<String>,
  /// Avatar sizes.
  pub avatar: Vec<Thumbnail>,
  /// The channel's notification bell.
  pub notifications: Option<ChannelNotifications>,
}

/// How much YouTube notifies the user about a subscribed channel's new videos.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NotificationLevel {
  /// "Toutes": every new video (`NOTIFICATIONS_ACTIVE`).
  All,
  /// "Personnalisées": the ones YouTube picks (`NOTIFICATIONS_NONE`, the default).
  Personalized,
  /// "Aucune" (`NOTIFICATIONS_OFF`).
  None,
}

/// A subscribed channel's notification bell (`subscriptionNotificationToggleButtonRenderer`):
/// the current level and the levels it offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelNotifications {
  /// The level now.
  pub level: NotificationLevel,
  /// The levels the menu offers, in its order, and the `params` of the
  /// `notification/modify_channel_preference` call setting each one, as the menu carries
  /// them. Some channels offer only "Personnalisées" and "Aucune".
  pub options: Vec<(NotificationLevel, String)>,
}

impl ChannelNotifications {
  /// The params setting `level`, if the menu offers it.
  pub fn params(&self, level: NotificationLevel) -> Option<&str> {
    self
      .options
      .iter()
      .find(|(l, _)| *l == level)
      .map(|(_, params)| params.as_str())
  }
}

/// A comment (top-level or reply).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comment {
  /// Comment id.
  pub id: String,
  /// Author display name (`@handle`).
  pub author: String,
  /// Author avatar URL.
  pub avatar: Option<String>,
  /// The author is the video's creator.
  pub by_creator: bool,
  /// Text, plain.
  pub text: String,
  /// Localized age, e.g. `il y a 7 ans`.
  pub published: String,
  /// Localized like count, empty when none.
  pub likes: String,
  /// Number of replies as shown (localized), empty when none.
  pub replies: String,
  /// Token of the replies, for [`crate::Client::comments`].
  pub replies_continuation: Option<String>,
}

/// A page of comments.
#[derive(Clone, Debug, Default)]
pub struct CommentPage {
  /// Comments in display order.
  pub comments: Vec<Comment>,
  /// Token of the next page.
  pub continuation: Option<String>,
  /// Localized total, e.g. `2,3 M commentaires` (first page only).
  pub count: Option<String>,
}

/// Like and dislike counts from Return YouTube Dislike (dislikes are its estimate).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Votes {
  /// Likes.
  pub likes: u64,
  /// Estimated dislikes.
  pub dislikes: u64,
}

/// A playlist of the user's library (`FEplaylist_aggregation`): their own playlists, saved
/// ones, "Watch later" (`WL`) and "Liked videos" (`LL`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaylistSummary {
  /// Playlist id (`PL…`, `WL`, `LL`…).
  pub id: String,
  /// Title.
  pub title: String,
  /// Localized video count as the badge shows it, e.g. `17 vidéos`.
  pub count: Option<String>,
  /// Localized details, e.g. `["Privée", "Playlist", "Mise à jour aujourd'hui"]`.
  pub metadata: Vec<String>,
  /// Thumbnail of its first video.
  pub thumbnails: Vec<Thumbnail>,
}

/// A playlist's page (`VL<id>`): its header and the first videos (100 at most).
#[derive(Clone, Debug, Default)]
pub struct Playlist {
  /// Playlist id.
  pub id: String,
  /// Title.
  pub title: String,
  /// Owner, e.g. `TeamFourStar`.
  pub owner: Option<String>,
  /// Localized video count, e.g. `69 vidéos`.
  pub count: Option<String>,
  /// Thumbnail of its first video.
  pub thumbnails: Vec<Thumbnail>,
  /// Playable videos in playlist order (deleted and private ones are left out). A video may
  /// appear twice.
  pub videos: Vec<VideoItem>,
  /// Token of the next videos, for [`crate::Client::playlist_more`].
  pub continuation: Option<String>,
}

/// A playlist the user can save a video to, from the "Save to playlist" dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaylistChoice {
  /// Playlist id (`WL`, `PL…`).
  pub id: String,
  /// Title.
  pub title: String,
  /// Whether the video is already in it.
  pub contains: bool,
  /// Only the user can see it (`PRIVATE`; `UNLISTED` and `PUBLIC` are not).
  pub private: bool,
}

/// A notification of the inbox (the title bar's bell on youtube.com).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
  /// Notification id.
  pub id: String,
  /// Text as YouTube writes it, e.g. `Asmongold TV a mis en ligne …` (spaces collapsed).
  pub message: String,
  /// Localized age, e.g. `il y a 1 heure`.
  pub sent: String,
  /// Whether the user has opened it.
  pub read: bool,
  /// The video it is about (a Short included).
  pub video_id: Option<String>,
  /// The video is a Short (`reelWatchEndpoint`).
  pub short: bool,
  /// The channel's avatar.
  pub avatar: Vec<Thumbnail>,
  /// The video's thumbnail.
  pub thumbnails: Vec<Thumbnail>,
  /// What [`crate::Client::mark_notification_read`] sends: the notification's
  /// `recordNotificationInteractionsEndpoint`, as a click on it records it.
  pub read_token: Option<String>,
}

/// A titled group of notifications ("Important", "Autres notifications").
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotificationSection {
  /// Title, empty for the notifications of a next page.
  pub title: String,
  /// Notifications, newest first within YouTube's grouping.
  pub items: Vec<Notification>,
}

/// A page of the inbox.
#[derive(Clone, Debug, Default)]
pub struct NotificationPage {
  /// Sections in display order. A next page has one untitled section, which continues the
  /// last one.
  pub sections: Vec<NotificationSection>,
  /// Token of the next page, for [`crate::Client::notifications_more`].
  pub continuation: Option<String>,
}
