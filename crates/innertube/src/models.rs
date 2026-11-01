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
