//! Hand-written models for the subset of the API Peeby uses. Unknown fields are ignored;
//! fields the spec marks nullable are `Option`, nullable collections default to empty.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{ItemId, UserId};

/// `GET /System/Info/Public`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PublicSystemInfo {
  /// Display name of the server.
  pub server_name: Option<String>,
  /// Server version, e.g. `12.3.0`.
  pub version: Option<String>,
  /// Product name, `Jellyfin Server`.
  pub product_name: Option<String>,
  /// Server id.
  pub id: Option<String>,
  /// False until the setup wizard has run.
  pub startup_wizard_completed: Option<bool>,
}

impl PublicSystemInfo {
  /// Major version reported by the server, if it parses.
  pub fn major_version(&self) -> Option<u32> {
    self.version.as_deref()?.split('.').next()?.parse().ok()
  }

  /// Whether the server meets the 12.0 minimum (ADR 0009).
  pub fn is_supported(&self) -> bool {
    self.major_version().is_some_and(|major| major >= 12)
  }
}

/// `POST /Users/AuthenticateByName` and `POST /Users/AuthenticateWithQuickConnect`.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AuthenticationResult {
  /// The logged-in user.
  pub user: Option<User>,
  /// Token to send in the `Authorization` header.
  pub access_token: Option<String>,
  /// Id of the server that issued the token.
  pub server_id: Option<String>,
}

impl std::fmt::Debug for AuthenticationResult {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("AuthenticationResult")
      .field("user", &self.user)
      .field(
        "access_token",
        &self.access_token.as_ref().map(|_| "<redacted>"),
      )
      .field("server_id", &self.server_id)
      .finish()
  }
}

/// `UserDto`, reduced.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct User {
  /// User id.
  pub id: UserId,
  /// Display name.
  pub name: Option<String>,
  /// Avatar image tag, if any.
  pub primary_image_tag: Option<String>,
}

/// Quick Connect state, from `Initiate` and `Connect`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct QuickConnectResult {
  /// True once the code has been approved from another device.
  pub authenticated: bool,
  /// Secret to poll with and to exchange for a token. Keep it private.
  pub secret: String,
  /// Code the user types on an already logged-in device.
  pub code: String,
}

/// Paged list, `BaseItemDtoQueryResult`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueryResult<T> {
  /// Items of this page.
  #[serde(default = "Vec::new")]
  pub items: Vec<T>,
  /// Total number of matching items.
  #[serde(default)]
  pub total_record_count: u32,
  /// Index of the first item of this page.
  #[serde(default)]
  pub start_index: u32,
}

/// `BaseItemDto`, reduced to what the UI needs.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct BaseItem {
  /// Item id.
  pub id: ItemId,
  /// Display name.
  pub name: Option<String>,
  /// Item kind.
  #[serde(rename = "Type")]
  pub kind: ItemKind,
  /// For libraries and views: what they contain.
  pub collection_type: Option<CollectionType>,
  /// Whether the item contains other items.
  pub is_folder: Option<bool>,
  /// Parent item id.
  pub parent_id: Option<ItemId>,
  /// Synopsis (requested with the `Overview` field).
  pub overview: Option<String>,
  /// Release year.
  pub production_year: Option<i32>,
  /// Duration in ticks (1 tick = 100 ns).
  pub run_time_ticks: Option<i64>,
  /// Community rating out of 10.
  pub community_rating: Option<f32>,
  /// Parental rating, e.g. `PG-13`.
  pub official_rating: Option<String>,
  /// Genres (requested with the `Genres` field).
  #[serde(default)]
  pub genres: Option<Vec<String>>,
  /// Episode or season number.
  pub index_number: Option<i32>,
  /// Season number of an episode.
  pub parent_index_number: Option<i32>,
  /// Series of an episode or season.
  pub series_id: Option<ItemId>,
  /// Series name of an episode or season.
  pub series_name: Option<String>,
  /// Season of an episode.
  pub season_id: Option<ItemId>,
  /// Number of children, for folders.
  pub child_count: Option<i32>,
  /// Width / height of the primary image.
  pub primary_image_aspect_ratio: Option<f64>,
  /// Image type → tag. Tags change when the image changes, so they key caches.
  #[serde(default)]
  pub image_tags: Option<HashMap<String, String>>,
  /// Backdrop image tags, by index.
  #[serde(default)]
  pub backdrop_image_tags: Option<Vec<String>>,
  /// Image type → (tag → BlurHash) for placeholders.
  #[serde(default)]
  pub image_blur_hashes: Option<HashMap<String, HashMap<String, Option<String>>>>,
  /// Per-user state (progress, watched, favorite).
  pub user_data: Option<UserItemData>,
  /// Item that holds the backdrop to show for this one (the series, for an episode).
  pub parent_backdrop_item_id: Option<ItemId>,
  /// Backdrop tags of `parent_backdrop_item_id`.
  #[serde(default)]
  pub parent_backdrop_image_tags: Option<Vec<String>>,
  /// Primary image tag of the series, for episodes and seasons.
  pub series_primary_image_tag: Option<String>,
  /// Item that holds the thumb to show for this one.
  pub parent_thumb_item_id: Option<ItemId>,
  /// Thumb tag of `parent_thumb_item_id`.
  pub parent_thumb_image_tag: Option<String>,
  /// Item that holds the logo to show for this one (the series, for an episode).
  pub parent_logo_item_id: Option<ItemId>,
  /// Logo tag of `parent_logo_item_id`.
  pub parent_logo_image_tag: Option<String>,
  /// First air or release date (ISO 8601).
  pub premiere_date: Option<String>,
  /// Series status, e.g. `Continuing` or `Ended`.
  pub status: Option<String>,
  /// Trickplay thumbnails, for videos (returned by `Items/{id}`).
  #[serde(default)]
  pub trickplay: Option<crate::media::TrickplayManifest>,
}

impl BaseItem {
  /// Tag of an image type, e.g. `Primary`.
  pub fn image_tag(&self, image_type: ImageType) -> Option<&str> {
    self
      .image_tags
      .as_ref()?
      .get(image_type.as_str())
      .map(String::as_str)
  }

  /// BlurHash of the current image of a type.
  pub fn blur_hash(&self, image_type: ImageType) -> Option<&str> {
    let tag = self.image_tag(image_type)?;
    self
      .image_blur_hashes
      .as_ref()?
      .get(image_type.as_str())?
      .get(tag)?
      .as_deref()
  }
}

/// `UserItemDataDto`, reduced.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct UserItemData {
  /// Resume position in ticks.
  #[serde(default)]
  pub playback_position_ticks: i64,
  /// Watched percentage, when in progress.
  pub played_percentage: Option<f64>,
  /// Unwatched children, for series and seasons.
  pub unplayed_item_count: Option<i32>,
  /// Fully watched.
  #[serde(default)]
  pub played: bool,
  /// Marked as favorite.
  #[serde(default)]
  pub is_favorite: bool,
}

/// `BaseItemKind`, with the kinds Peeby handles; anything else is `Other`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ItemKind {
  #[allow(missing_docs)]
  Movie,
  #[allow(missing_docs)]
  Series,
  #[allow(missing_docs)]
  Season,
  #[allow(missing_docs)]
  Episode,
  #[allow(missing_docs)]
  Video,
  #[allow(missing_docs)]
  BoxSet,
  #[allow(missing_docs)]
  Folder,
  #[allow(missing_docs)]
  CollectionFolder,
  #[allow(missing_docs)]
  UserView,
  #[allow(missing_docs)]
  Playlist,
  #[allow(missing_docs)]
  Audio,
  #[allow(missing_docs)]
  MusicAlbum,
  /// Any kind Peeby does not handle specially.
  #[serde(other)]
  Other,
}

impl ItemKind {
  /// Name used by the API in `includeItemTypes`.
  pub fn as_str(self) -> &'static str {
    match self {
      Self::Movie => "Movie",
      Self::Series => "Series",
      Self::Season => "Season",
      Self::Episode => "Episode",
      Self::Video => "Video",
      Self::BoxSet => "BoxSet",
      Self::Folder => "Folder",
      Self::CollectionFolder => "CollectionFolder",
      Self::UserView => "UserView",
      Self::Playlist => "Playlist",
      Self::Audio => "Audio",
      Self::MusicAlbum => "MusicAlbum",
      Self::Other => "Other",
    }
  }
}

/// `CollectionType` of a library.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CollectionType {
  #[allow(missing_docs)]
  Movies,
  #[allow(missing_docs)]
  TvShows,
  #[allow(missing_docs)]
  Music,
  #[allow(missing_docs)]
  MusicVideos,
  #[allow(missing_docs)]
  HomeVideos,
  #[allow(missing_docs)]
  BoxSets,
  #[allow(missing_docs)]
  Playlists,
  #[allow(missing_docs)]
  Folders,
  /// Any other type (books, photos, live TV…).
  #[serde(other)]
  Other,
}

impl CollectionType {
  /// Whether Peeby can browse and play this library (video content).
  pub fn is_video(self) -> bool {
    matches!(
      self,
      Self::Movies
        | Self::TvShows
        | Self::MusicVideos
        | Self::HomeVideos
        | Self::BoxSets
        | Self::Folders
    )
  }
}

/// `ImageType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum ImageType {
  Primary,
  Backdrop,
  Logo,
  Thumb,
  Banner,
}

impl ImageType {
  /// Name used in image URLs and `ImageTags` keys.
  pub fn as_str(self) -> &'static str {
    match self {
      Self::Primary => "Primary",
      Self::Backdrop => "Backdrop",
      Self::Logo => "Logo",
      Self::Thumb => "Thumb",
      Self::Banner => "Banner",
    }
  }
}
