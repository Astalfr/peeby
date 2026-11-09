use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::Duration;

use serde_json::{Value, json};

use crate::auth::{Cookies, ORIGIN, ROTATE_COOKIES_BODY, ROTATE_COOKIES_URL, parse_rotation_delay};
use crate::error::{Error, Result};
use crate::hls;
use crate::live::LiveProxy;
use crate::models::{
  Channel, CommentPage, Feed, LikeStatus, Live, LiveChatPage, NotificationPage, Playlist,
  PlaylistChoice, PlaylistSummary, Related, SearchPage, SearchResult, ShortsPage, Stream, Tracking,
  VideoItem, Votes, WatchPage, Watchtime,
};
use crate::parse::{
  find_continuation, is_mix, parse_account_handle, parse_captions, parse_channel_search,
  parse_channels, parse_comment_action, parse_comment_translation, parse_comments, parse_feed,
  parse_live, parse_live_chat, parse_live_formats, parse_members_only, parse_mix, parse_mixes,
  parse_notifications, parse_oembed_title, parse_playlist, parse_playlist_choices,
  parse_playlist_page, parse_playlists, parse_search, parse_shorts_seed, parse_shorts_sequence,
  parse_storyboard, parse_subscribed, parse_subscriptions, parse_tracking, parse_unseen_count,
  parse_watch, tracking_param,
};

/// "My Mix": the account's own mix, in no feed.
const MY_MIX: &str = "RDMM";
/// Pages of the home feed [`Client::mixes`] reads: about one mix each.
const MIX_PAGES: usize = 5;

/// WEB client version used until [`Client::refresh_client_version`] reads the current one from
/// the page's ytcfg. YouTube ships a new one every week; old ones keep working for months.
const WEB_CLIENT_VERSION: &str = "2.20261029.11.00";
const WEB_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";

/// VISIONOS client, values from yt-dlp 2026.10.26 (`yt_dlp/extractor/youtube/_base.py`), the
/// source of truth for them. It returns a server-signed HLS master playlist: no JS challenge,
/// no `available_at` delay (ADR 0003).
const VISIONOS_CLIENT_NAME_ID: &str = "101";
const VISIONOS_CLIENT_VERSION: &str = "1.02";
/// Safari UA of the VISIONOS client; HLS playlists and segments are fetched with it too.
pub const VISIONOS_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

/// Original titles looked up at once: a playlist's first page holds 100 videos.
const TITLE_BATCH: usize = 32;
/// Original titles kept; the cache starts over beyond.
const TITLES_KEPT: usize = 5000;
/// An oEmbed lookup that takes longer keeps the translated title.
const TITLE_TIMEOUT: Duration = Duration::from_secs(3);
/// Longest wait for an image, from connecting to its last byte.
const IMAGE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Default)]
struct State {
  /// visitorData of the WEB session (`X-Goog-Visitor-Id`), from the first response or ytcfg.
  visitor_data: Option<String>,
  /// visitorData for VISIONOS, always from an anonymous request: most videos answer
  /// `LOGIN_REQUIRED` without one, and one tied to the signed-in session does not help.
  resolver_visitor: Option<String>,
  web_version: Option<String>,
  /// Original titles by video id; `None` when oEmbed has none (embedding disabled, private).
  titles: HashMap<String, Option<String>>,
}

/// Saves the whole jar when YouTube rotates a value ([`Client::new`]). Blocking: it runs on the
/// network runtime, the jar being small.
pub type SaveCookies = Arc<dyn Fn(&Cookies) -> std::io::Result<()> + Send + Sync>;

/// A [`SaveCookies`] that writes a plain Netscape file at `path` (next to it then renamed, so a
/// crash never leaves a truncated jar). The app encrypts its own jar instead (peeby ADR 0022).
pub fn save_to_file(path: PathBuf) -> SaveCookies {
  Arc::new(move |cookies| {
    let temporary = path.with_extension("txt.tmp");
    std::fs::write(&temporary, cookies.to_netscape())?;
    std::fs::rename(&temporary, &path)
  })
}

/// Session cookies and where the rotated values are saved.
struct Session {
  cookies: Mutex<Cookies>,
  save: Option<SaveCookies>,
}

/// InnerTube client. Cheap to clone; clones share the session state.
#[derive(Clone)]
pub struct Client {
  http: reqwest::Client,
  session: Option<Arc<Session>>,
  language: Arc<(String, String)>,
  /// Titles in the videos' own language ([`Client::with_original_titles`]).
  original_titles: bool,
  state: Arc<Mutex<State>>,
}

impl Client {
  /// A client, signed in when `cookies` are given. `hl`/`gl` pick the language and region of
  /// texts and recommendations (e.g. `fr`, `FR`). YouTube rotates some cookies in its
  /// responses: when `save` is given, it gets the updated jar, otherwise the session is
  /// invalidated after a while.
  pub fn new(
    cookies: Option<Cookies>,
    save: Option<SaveCookies>,
    hl: &str,
    gl: &str,
  ) -> Result<Self> {
    let http = reqwest::Client::builder()
      .user_agent(WEB_USER_AGENT)
      .build()?;
    Ok(Self {
      http,
      session: cookies.map(|cookies| {
        Arc::new(Session {
          cookies: Mutex::new(cookies),
          save,
        })
      }),
      language: Arc::new((hl.to_owned(), gl.to_owned())),
      original_titles: false,
      state: Arc::default(),
    })
  }

  /// With `on`, the videos of feeds, searches, playlists and watch pages keep the title of
  /// their own language: YouTube translates titles into `hl` (the creator's translations or
  /// its automatic ones) and no InnerTube response carries the original, so each video costs
  /// one anonymous oEmbed request, cached. Applies to this client and its later clones.
  pub fn with_original_titles(mut self, on: bool) -> Self {
    self.original_titles = on;
    self
  }

  /// Whether requests carry session cookies.
  pub fn signed_in(&self) -> bool {
    self.session.is_some()
  }

  /// Home recommendations. Signed out, YouTube returns no videos, only a
  /// [`Feed::message`]. Signed in but answered as signed out: [`Error::NotLoggedIn`].
  pub async fn home(&self) -> Result<Feed> {
    let response = self
      .web("browse", json!({ "browseId": "FEwhat_to_watch" }), true)
      .await?;
    self.feed(&response).await
  }

  /// Latest videos of the subscribed channels. Needs cookies.
  pub async fn subscriptions(&self) -> Result<Feed> {
    self.require_session()?;
    let response = self
      .web("browse", json!({ "browseId": "FEsubscriptions" }), true)
      .await?;
    self
      .untranslated(self.checked(parse_subscriptions(&response))?)
      .await
  }

  /// Next page of the subscriptions feed.
  pub async fn subscriptions_more(&self, token: &str) -> Result<Feed> {
    let response = self
      .web("browse", json!({ "continuation": token }), true)
      .await?;
    self
      .untranslated(self.checked(parse_subscriptions(&response))?)
      .await
  }

  /// The account's watch history, most recent first (its next pages through
  /// [`Client::continuation`]). Needs cookies. With the history off, no videos and a
  /// [`Feed::message`].
  pub async fn history(&self) -> Result<Feed> {
    self.require_session()?;
    let response = self
      .web("browse", json!({ "browseId": "FEhistory" }), true)
      .await?;
    self.feed(&response).await
  }

  /// All subscribed channels, in YouTube's order. Needs cookies.
  pub async fn subscribed_channels(&self) -> Result<Vec<Channel>> {
    self.require_session()?;
    let response = self
      .web("browse", json!({ "browseId": "FEchannels" }), true)
      .await?;
    if parse_feed(&response).logged_in {
      Ok(parse_channels(&response))
    } else {
      Err(Error::NotLoggedIn)
    }
  }

  /// The user's playlists, "Watch later" and "Liked videos" first. Needs cookies.
  pub async fn playlists(&self) -> Result<Vec<PlaylistSummary>> {
    self.require_session()?;
    let response = self
      .web(
        "browse",
        json!({ "browseId": "FEplaylist_aggregation" }),
        true,
      )
      .await?;
    if tracking_param(&response, "logged_in").as_deref() == Some("1") {
      Ok(parse_playlists(&response))
    } else {
      Err(Error::NotLoggedIn)
    }
  }

  /// The user's mixes: "My Mix" (`RDMM`), then those of the first pages of the home feed and of
  /// the playlists library, each once. YouTube has no page of them: it shows about one mix per
  /// home page. Needs cookies.
  pub async fn mixes(&self) -> Result<Vec<PlaylistSummary>> {
    self.require_session()?;
    let mut mixes = Vec::new();
    // "My Mix" is in no feed: its card is its first video.
    let my_mix = self.playlist(MY_MIX).await?;
    mixes.push(PlaylistSummary {
      id: my_mix.id,
      title: my_mix.title,
      count: None,
      channel: None,
      channel_id: None,
      metadata: Vec::new(),
      thumbnails: my_mix.thumbnails,
    });
    let mut response = self
      .web("browse", json!({ "browseId": "FEwhat_to_watch" }), true)
      .await?;
    self.check_session(&response)?;
    for page in 0..MIX_PAGES {
      mixes.extend(parse_mixes(&response));
      let Some(token) = find_continuation(&response).filter(|_| page + 1 < MIX_PAGES) else {
        break;
      };
      response = self
        .web("browse", json!({ "continuation": token }), true)
        .await?;
    }
    let library = self
      .web(
        "browse",
        json!({ "browseId": "FEplaylist_aggregation" }),
        true,
      )
      .await?;
    mixes.extend(parse_mixes(&library));
    let mut seen = std::collections::HashSet::new();
    mixes.retain(|m| seen.insert(m.id.clone()));
    Ok(mixes)
  }

  /// A playlist's header and first videos (100 at most), or a mix's videos ([`is_mix`]).
  /// Private playlists, "Watch later", "Liked videos" and "My Mix" need cookies.
  pub async fn playlist(&self, id: &str) -> Result<Playlist> {
    if is_mix(id) {
      let response = self.web("next", json!({ "playlistId": id }), true).await?;
      self.check_session(&response)?;
      let mut mix = parse_mix(id, &response);
      self.untranslate(mix.videos.iter_mut()).await;
      return Ok(mix);
    }
    let response = self
      .web("browse", json!({ "browseId": format!("VL{id}") }), true)
      .await?;
    self.check_session(&response)?;
    let mut playlist = parse_playlist(id, &response);
    self.untranslate(playlist.videos.iter_mut()).await;
    Ok(playlist)
  }

  /// The next videos of a playlist, from [`Playlist::continuation`] or a previous page.
  pub async fn playlist_more(&self, token: &str) -> Result<Feed> {
    let response = self
      .web("browse", json!({ "continuation": token }), true)
      .await?;
    self.check_session(&response)?;
    self.untranslated(parse_playlist_page(&response)).await
  }

  /// The playlists `video_id` can be saved to, and whether each one holds it already. Needs
  /// cookies.
  pub async fn playlist_choices(&self, video_id: &str) -> Result<Vec<PlaylistChoice>> {
    self.require_session()?;
    let body = json!({ "videoIds": [video_id], "excludeWatchLater": false });
    let response = self.web("playlist/get_add_to_playlist", body, true).await?;
    self.check_session(&response)?;
    Ok(parse_playlist_choices(&response))
  }

  /// Adds `video_id` at the end of a playlist (`WL` for "Watch later"). Needs cookies.
  pub async fn add_to_playlist(&self, playlist_id: &str, video_id: &str) -> Result<()> {
    let action = json!({ "action": "ACTION_ADD_VIDEO", "addedVideoId": video_id });
    self.edit_playlist(playlist_id, action).await
  }

  /// Removes `video_id` from a playlist. Needs cookies.
  pub async fn remove_from_playlist(&self, playlist_id: &str, video_id: &str) -> Result<()> {
    let action = json!({ "action": "ACTION_REMOVE_VIDEO_BY_VIDEO_ID", "removedVideoId": video_id });
    self.edit_playlist(playlist_id, action).await
  }

  /// Removes one entry from a playlist the user can edit ([`crate::VideoItem::set_video_id`]),
  /// as the playlist page's own menu does. Needs cookies.
  pub async fn remove_playlist_entry(&self, playlist_id: &str, set_video_id: &str) -> Result<()> {
    let action = json!({ "action": "ACTION_REMOVE_VIDEO", "setVideoId": set_video_id });
    self.edit_playlist(playlist_id, action).await
  }

  /// `browse/edit_playlist` with one action, the body the "Save to playlist" dialog's
  /// endpoints carry (see `tests/fixtures/add_to_playlist.json`). No response of it is saved
  /// yet: an HTTP success counts as done unless the response has a `status` other than
  /// `STATUS_SUCCEEDED`.
  async fn edit_playlist(&self, playlist_id: &str, action: Value) -> Result<()> {
    self.require_session()?;
    let body = json!({ "playlistId": playlist_id, "actions": [action] });
    let response = self.web("browse/edit_playlist", body, true).await?;
    match response["status"].as_str() {
      None | Some("STATUS_SUCCEEDED") => Ok(()),
      Some(status) => Err(Error::Parse {
        endpoint: "browse/edit_playlist",
        detail: format!("status {status}"),
      }),
    }
  }

  /// "Videos" tab of a channel, newest first.
  pub async fn channel_videos(&self, channel_id: &str) -> Result<Feed> {
    // Protobuf selecting the "Videos" tab, as the web client sends it.
    let body = json!({ "browseId": channel_id, "params": "EgZ2aWRlb3PyBgQKAjoA" });
    let response = self.web("browse", body, true).await?;
    self.feed(&response).await
  }

  /// A channel's videos matching `query`, as its "Search" tab shows them (YouTube's order). The
  /// next pages come from [`Client::channel_search_more`].
  pub async fn channel_search(&self, channel_id: &str, query: &str) -> Result<Feed> {
    // Protobuf selecting the "Search" tab, as the web client sends it.
    let body = json!({ "browseId": channel_id, "params": "EgZzZWFyY2jyBgQKAloA", "query": query });
    let response = self.web("browse", body, true).await?;
    self
      .untranslated(self.checked(parse_channel_search(&response))?)
      .await
  }

  /// The next page of a channel's search, from its [`Feed::continuation`].
  pub async fn channel_search_more(&self, token: &str) -> Result<Feed> {
    let response = self
      .web("browse", json!({ "continuation": token }), true)
      .await?;
    self
      .untranslated(self.checked(parse_channel_search(&response))?)
      .await
  }

  /// A page of comments or of replies. `token` comes from
  /// [`WatchPage::comments_continuation`], [`CommentPage::continuation`] or
  /// [`crate::Comment::replies_continuation`].
  pub async fn comments(&self, token: &str) -> Result<CommentPage> {
    let response = self.comments_response(token).await?;
    Ok(parse_comments(&response))
  }

  /// The whole answer behind [`Client::comments`], for the `comments` example's fixtures.
  pub async fn comments_response(&self, token: &str) -> Result<Value> {
    let response = self
      .web("next", json!({ "continuation": token }), true)
      .await?;
    self.check_session(&response)?;
    Ok(response)
  }

  /// Sets the user's rating of `video_id`: `like/like`, `like/dislike`, or `like/removelike`
  /// for [`LikeStatus::Indifferent`]. `params` come from [`WatchPage::like_params`], as the
  /// page's buttons send them. Needs cookies. No response of these endpoints is saved yet: an
  /// HTTP success counts as done.
  pub async fn rate(&self, video_id: &str, status: LikeStatus, params: Option<&str>) -> Result<()> {
    self.require_session()?;
    let endpoint = match status {
      LikeStatus::Like => "like/like",
      LikeStatus::Dislike => "like/dislike",
      LikeStatus::Indifferent => "like/removelike",
    };
    let mut body = json!({ "target": { "videoId": video_id } });
    if let Some(params) = params {
      body["params"] = params.into();
    }
    self.web(endpoint, body, true).await?;
    Ok(())
  }

  /// Likes or dislikes a comment, or removes that rating: `action` comes from
  /// [`crate::CommentActions::get`], as the comment's thumbs send it
  /// (`comment/perform_comment_action`). Needs cookies. The answer's `actionResults` must say
  /// `STATUS_SUCCEEDED`.
  pub async fn rate_comment(&self, action: &str) -> Result<()> {
    let response = self.rate_comment_response(action).await?;
    match parse_comment_action(&response) {
      Some(status) if status == "STATUS_SUCCEEDED" => Ok(()),
      status => Err(Error::Parse {
        endpoint: "comment/perform_comment_action",
        detail: format!("status: {status:?}"),
      }),
    }
  }

  /// The whole answer behind [`Client::rate_comment`], for the `comments` example's fixtures.
  pub async fn rate_comment_response(&self, action: &str) -> Result<Value> {
    self.require_session()?;
    let body = json!({ "actions": [action] });
    self.web("comment/perform_comment_action", body, true).await
  }

  /// Comment `comment_id` of `video_id`, whose text is `text`, translated by Google into the
  /// client's language (`hl`), as the web client's "Translate" button asks for it
  /// (`comment/perform_comment_action`). No answer gives that action: it is built here. Needs no
  /// sign-in and writes nothing to the account.
  pub async fn translate_comment(
    &self,
    video_id: &str,
    comment_id: &str,
    text: &str,
  ) -> Result<String> {
    let response = self
      .translate_comment_response(video_id, comment_id, text)
      .await?;
    let failed = |detail: String| Error::Parse {
      endpoint: "comment/perform_comment_action",
      detail,
    };
    match parse_comment_action(&response) {
      Some(status) if status == "STATUS_SUCCEEDED" => {
        parse_comment_translation(&response).ok_or_else(|| failed("no translation".into()))
      }
      status => Err(failed(format!("status: {status:?}"))),
    }
  }

  /// The whole answer behind [`Client::translate_comment`], for fixtures.
  pub async fn translate_comment_response(
    &self,
    video_id: &str,
    comment_id: &str,
    text: &str,
  ) -> Result<Value> {
    let action =
      crate::proto::translate_comment_action(video_id, comment_id, text, &self.language.0);
    let body = json!({ "actions": [action] });
    self.web("comment/perform_comment_action", body, true).await
  }

  /// Sets how much YouTube notifies the user about a subscribed channel. `params` come from
  /// [`crate::ChannelNotifications::params`], as the bell's menu sends them. Needs cookies. No
  /// response of this endpoint is saved yet: an HTTP success counts as done.
  pub async fn set_channel_notifications(&self, params: &str) -> Result<()> {
    self.require_session()?;
    let body = json!({ "params": params });
    self
      .web("notification/modify_channel_preference", body, true)
      .await?;
    Ok(())
  }

  /// Subscribes the user to `channel_id` (`subscribe`), or unsubscribes them. `params` come
  /// from [`crate::Subscription::params`], as the page's button sends them. Needs cookies. The
  /// answer's `updateSubscribeButtonAction` must show the state asked for. A watch page read
  /// right after can still show the former state for a while (seen after an unsubscription,
  /// 2026-11-03).
  pub async fn set_subscribed(
    &self,
    channel_id: &str,
    subscribe: bool,
    params: Option<&str>,
  ) -> Result<()> {
    self.require_session()?;
    let endpoint = if subscribe {
      "subscription/subscribe"
    } else {
      "subscription/unsubscribe"
    };
    let mut body = json!({ "channelIds": [channel_id] });
    if let Some(params) = params {
      body["params"] = params.into();
    }
    let response = self.web(endpoint, body, true).await?;
    match parse_subscribed(&response, channel_id) {
      Some(state) if state == subscribe => Ok(()),
      state => Err(Error::Parse {
        endpoint,
        detail: format!("subscribed: {state:?}"),
      }),
    }
  }

  /// The first page of the notifications inbox, as youtube.com's bell shows it. Needs cookies.
  /// youtube.com reads it with `browse` (`FEnotifications_inbox`): the older
  /// `notification/get_notification_menu` now answers an empty inbox (checked 2026-11-01).
  pub async fn notifications(&self) -> Result<NotificationPage> {
    self.require_session()?;
    let response = self
      .web(
        "browse",
        json!({ "browseId": "FEnotifications_inbox" }),
        true,
      )
      .await?;
    self.check_session(&response)?;
    Ok(parse_notifications(&response))
  }

  /// The next notifications, from [`NotificationPage::continuation`]. Needs cookies.
  pub async fn notifications_more(&self, token: &str) -> Result<NotificationPage> {
    self.require_session()?;
    let response = self
      .web("browse", json!({ "continuation": token }), true)
      .await?;
    self.check_session(&response)?;
    Ok(parse_notifications(&response))
  }

  /// The account's channel handle (`@name`), as its live chat messages show it; `None` for an
  /// account without a channel. Needs cookies.
  pub async fn account_handle(&self) -> Result<Option<String>> {
    Ok(parse_account_handle(&self.account_menu().await?))
  }

  /// `account/account_menu` as YouTube answers it (the account's name, handle and picture):
  /// for a fixture (the `account` example). Needs cookies.
  pub async fn account_menu(&self) -> Result<Value> {
    self.require_session()?;
    let response = self.web("account/account_menu", json!({}), true).await?;
    self.check_session(&response)?;
    Ok(response)
  }

  /// How many notifications arrived since the user last opened the inbox (youtube.com's
  /// badge). Needs cookies.
  pub async fn unseen_notifications(&self) -> Result<u32> {
    self.require_session()?;
    let response = self
      .web("notification/get_unseen_count", json!({}), true)
      .await?;
    self.check_session(&response)?;
    parse_unseen_count(&response).ok_or(Error::Parse {
      endpoint: "notification/get_unseen_count",
      detail: "unseenCount missing".into(),
    })
  }

  /// Records a click on a notification, which marks it read. `token` is
  /// [`crate::Notification::read_token`]. Needs cookies. No response of this endpoint is saved
  /// yet: an HTTP success counts as done.
  pub async fn mark_notification_read(&self, token: &str) -> Result<()> {
    self.require_session()?;
    let body = json!({ "serializedRecordNotificationInteractionsRequest": token });
    self
      .web("notification/record_interactions", body, true)
      .await?;
    Ok(())
  }

  /// Likes and estimated dislikes from Return YouTube Dislike.
  pub async fn votes(&self, video_id: &str) -> Result<Votes> {
    let response = self
      .http
      .get(format!(
        "https://returnyoutubedislikeapi.com/votes?videoId={video_id}"
      ))
      .send()
      .await?;
    if !response.status().is_success() {
      return Err(Error::Status {
        status: response.status().as_u16(),
        endpoint: "returnyoutubedislike",
      });
    }
    let value: Value = response.json().await?;
    match (value["likes"].as_u64(), value["dislikes"].as_u64()) {
      (Some(likes), Some(dislikes)) => Ok(Votes { likes, dislikes }),
      _ => Err(Error::Parse {
        endpoint: "returnyoutubedislike",
        detail: "likes or dislikes missing".into(),
      }),
    }
  }

  /// Videos and channels matching `query`.
  pub async fn search(&self, query: &str) -> Result<SearchPage> {
    let response = self.web("search", json!({ "query": query }), true).await?;
    self.check_session(&response)?;
    Ok(self.untranslated_search(parse_search(&response)).await)
  }

  /// The next page of a search, from [`SearchPage::continuation`].
  pub async fn search_more(&self, token: &str) -> Result<SearchPage> {
    let response = self
      .web("search", json!({ "continuation": token }), true)
      .await?;
    self.check_session(&response)?;
    Ok(self.untranslated_search(parse_search(&response)).await)
  }

  /// Next page of a feed.
  pub async fn continuation(&self, token: &str) -> Result<Feed> {
    let response = self
      .web("browse", json!({ "continuation": token }), true)
      .await?;
    self.feed(&response).await
  }

  /// The Shorts feed: its first Short and the token of the next ones ([`Client::shorts_more`]).
  /// Personalized when signed in; signed in but answered as signed out: [`Error::NotLoggedIn`].
  pub async fn shorts(&self) -> Result<ShortsPage> {
    // As the web client asks from its "Shorts" entry: no video ("seedless"), `params` and
    // `inputType` from that entry's `reelWatchEndpoint`, without the player response.
    let body = json!({
      "params": "CA8%3D",
      "inputType": "REEL_WATCH_INPUT_TYPE_SEEDLESS",
      "disablePlayerResponse": true,
    });
    let response = self.web("reel/reel_item_watch", body, true).await?;
    self.check_session(&response)?;
    Ok(parse_shorts_seed(&response))
  }

  /// The next Shorts of the feed after `token` ([`ShortsPage::continuation`]).
  pub async fn shorts_more(&self, token: &str) -> Result<ShortsPage> {
    let response = self
      .web(
        "reel/reel_watch_sequence",
        json!({ "sequenceParams": token }),
        true,
      )
      .await?;
    self.check_session(&response)?;
    Ok(parse_shorts_sequence(&response))
  }

  /// Watch page: metadata, related videos, comments token.
  pub async fn watch(&self, video_id: &str) -> Result<WatchPage> {
    let response = self
      .web("next", json!({ "videoId": video_id }), true)
      .await?;
    let mut page = parse_watch(video_id, &response);
    let WatchPage {
      id, title, related, ..
    } = &mut page;
    let titles = std::iter::once((id.as_str(), title))
      .chain(related.iter_mut().filter_map(|r| match r {
        Related::Video(v) => Some((v.id.as_str(), &mut v.title)),
        Related::Playlist(_) => None,
      }))
      .collect();
    self.untranslate_titles(titles).await;
    Ok(page)
  }

  /// The page of a live's chat after `continuation` ([`WatchPage::live_chat`], then each page's
  /// own). Signed in when cookies are set, so the rotated ones are kept.
  pub async fn live_chat(&self, continuation: &str) -> Result<LiveChatPage> {
    let response = self
      .web(
        "live_chat/get_live_chat",
        json!({ "continuation": continuation }),
        true,
      )
      .await?;
    Ok(parse_live_chat(&response))
  }

  /// Best HLS stream up to `max_height` pixels. Takes about 0.2 s; the URLs stay valid for
  /// 6 hours. Retries once with a fresh visitorData on `LOGIN_REQUIRED`.
  pub async fn stream(&self, video_id: &str, max_height: u32) -> Result<Stream> {
    match self.visionos_stream(video_id, max_height).await {
      Err(Error::Unplayable { status, .. }) if status == "LOGIN_REQUIRED" => {
        tracing::debug!("VISIONOS: LOGIN_REQUIRED, retrying with a new visitorData");
        self.state().resolver_visitor = None;
        self.visionos_stream(video_id, max_height).await
      }
      result => result,
    }
  }

  /// Reads the current WEB client version and a visitorData from the home page's ytcfg.
  /// Call it once in the background at startup.
  pub async fn refresh_client_version(&self) -> Result<()> {
    let mut request = self.http.get(ORIGIN);
    if let Some(session) = &self.session {
      request = request.header("Cookie", lock(&session.cookies).header());
    }
    let response = request.send().await?;
    self.keep_rotated_cookies(&response);
    if !response.status().is_success() {
      return Err(Error::Status {
        status: response.status().as_u16(),
        endpoint: "ytcfg",
      });
    }
    let html = response.text().await?;
    let version = ytcfg_string(&html, "INNERTUBE_CLIENT_VERSION").ok_or(Error::Parse {
      endpoint: "ytcfg",
      detail: "INNERTUBE_CLIENT_VERSION missing".into(),
    })?;
    let mut state = self.state();
    tracing::debug!("WEB client version {version}");
    state.web_version = Some(version);
    if state.visitor_data.is_none() {
      state.visitor_data = ytcfg_string(&html, "VISITOR_DATA");
    }
    Ok(())
  }

  /// Where to report the playback of `video_id` to the account: a signed-in WEB `player`
  /// request, for its stats URLs only (its streams are not used). VISIONOS's are an anonymous
  /// visitor's: they write no history. Signed in only.
  pub async fn tracking(&self, video_id: &str) -> Result<Tracking> {
    self.require_session()?;
    let body = json!({ "videoId": video_id, "contentCheckOk": true, "racyCheckOk": true });
    let player = self.web("player", body, true).await?;
    self.check_session(&player)?;
    let version = self
      .state()
      .web_version
      .clone()
      .unwrap_or_else(|| WEB_CLIENT_VERSION.to_owned());
    parse_tracking(&player, &version).ok_or(Error::Parse {
      endpoint: "player",
      detail: "no playbackTracking".into(),
    })
  }

  /// Why VISIONOS refused `video_id`, when it is for the channel's members: `Err(MembersOnly)`
  /// if the WEB `player` (signed in when cookies are set) offers the membership instead of the
  /// video, `Ok(())` when it would play (the account is a member: yt-dlp, with the cookies, can
  /// play it) or is refused for another reason.
  pub async fn check_members_only(&self, video_id: &str) -> Result<()> {
    let body = json!({ "videoId": video_id, "contentCheckOk": true, "racyCheckOk": true });
    let player = self.web("player", body, true).await?;
    self.check_session(&player)?;
    if parse_members_only(&player) {
      return Err(Error::MembersOnly {
        signed_in: self.session.is_some(),
      });
    }
    Ok(())
  }

  /// Reports the start of playback at `position` seconds: the video goes to the top of the
  /// account's history (when the history is on).
  pub async fn report_playback(&self, tracking: &Tracking, position: f64) -> Result<()> {
    self.ping(&tracking.playback_url(position)).await
  }

  /// Reports a watched segment: its end becomes the video's resume position on every device.
  pub async fn report_watchtime(&self, tracking: &Tracking, segment: Watchtime) -> Result<()> {
    self.ping(&tracking.watchtime_url(segment)).await
  }

  /// A stats ping (`s.youtube.com/api/stats/…`): a GET with the session cookies, answered 204
  /// with no body. The URL carries session ids: it appears in no log or error.
  async fn ping(&self, url: &str) -> Result<()> {
    let session = self.session.as_ref().ok_or(Error::SignInRequired)?;
    let cookies = lock(&session.cookies).header();
    let request = self
      .http
      .get(url)
      .header("Origin", ORIGIN)
      .header("Cookie", cookies);
    let response = request.send().await?;
    self.keep_rotated_cookies(&response);
    if !response.status().is_success() {
      return Err(Error::Status {
        status: response.status().as_u16(),
        endpoint: "stats",
      });
    }
    Ok(())
  }

  /// Rotates the session's `__Secure-1PSIDTS`/`__Secure-3PSIDTS`, as youtube.com's hidden
  /// `RotateCookiesPage` frame does in a browser: the new values are saved like every rotation.
  /// Returns when to call it again (YouTube's `identity.hfcr`, 10 minutes so far). A 401 means
  /// the session is gone: [`Error::NotLoggedIn`]. Signed in only.
  pub async fn rotate_cookies(&self) -> Result<Duration> {
    /// When the answer gives no delay: the one it gave on 2026-11-09.
    const DEFAULT: Duration = Duration::from_secs(600);
    let session = self.session.as_ref().ok_or(Error::SignInRequired)?;
    let cookies = lock(&session.cookies).header();
    let response = self
      .http
      .post(ROTATE_COOKIES_URL)
      .header("Origin", "https://accounts.youtube.com")
      .header("Content-Type", "application/json")
      .header("Cookie", cookies)
      .body(ROTATE_COOKIES_BODY)
      .send()
      .await?;
    self.keep_rotated_cookies(&response);
    match response.status() {
      status if status.is_success() => {}
      reqwest::StatusCode::UNAUTHORIZED => return Err(Error::NotLoggedIn),
      status => {
        return Err(Error::Status {
          status: status.as_u16(),
          endpoint: "RotateCookies",
        });
      }
    }
    let body = response.text().await?;
    Ok(parse_rotation_delay(&body).unwrap_or(DEFAULT))
  }

  /// Downloads an image (thumbnails, avatars).
  pub async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>> {
    // A stalled download would hold one of the image store's few download slots for good.
    let response = self.http.get(url).timeout(IMAGE_TIMEOUT).send().await?;
    if !response.status().is_success() {
      return Err(Error::Status {
        status: response.status().as_u16(),
        endpoint: "image",
      });
    }
    Ok(response.bytes().await?.to_vec())
  }

  fn state(&self) -> std::sync::MutexGuard<'_, State> {
    // A poisoned lock only means a panic elsewhere; the state stays usable.
    self
      .state
      .lock()
      .unwrap_or_else(std::sync::PoisonError::into_inner)
  }

  /// Applies the `Set-Cookie` headers of a signed-in response and writes the jar back if a
  /// value changed. The file is small: written synchronously, on the network runtime.
  fn keep_rotated_cookies(&self, response: &reqwest::Response) {
    let Some(session) = &self.session else { return };
    let mut cookies = lock(&session.cookies);
    let mut changed = false;
    for header in response.headers().get_all(reqwest::header::SET_COOKIE) {
      if let Ok(header) = header.to_str() {
        changed |= cookies.apply_set_cookie(header);
      }
    }
    if !changed {
      return;
    }
    tracing::debug!("session cookies rotated");
    if let Some(save) = &session.save
      && let Err(e) = save(&cookies)
    {
      tracing::warn!("cannot save the rotated cookies: {e}");
    }
  }

  fn require_session(&self) -> Result<()> {
    if self.session.is_some() {
      Ok(())
    } else {
      Err(Error::SignInRequired)
    }
  }

  async fn feed(&self, response: &Value) -> Result<Feed> {
    self.untranslated(self.checked(parse_feed(response))?).await
  }

  /// `feed` with its original titles, when asked for.
  async fn untranslated(&self, mut feed: Feed) -> Result<Feed> {
    self.untranslate(feed.items.iter_mut()).await;
    Ok(feed)
  }

  async fn untranslated_search(&self, mut page: SearchPage) -> SearchPage {
    let videos = page.results.iter_mut().filter_map(|result| match result {
      SearchResult::Video(video) => Some(video),
      SearchResult::Channel(_) | SearchResult::Playlist(_) => None,
    });
    self.untranslate(videos).await;
    page
  }

  /// With cookies, a feed served as signed out means the session is gone.
  fn checked(&self, feed: Feed) -> Result<Feed> {
    if self.session.is_some() && !feed.logged_in {
      return Err(Error::NotLoggedIn);
    }
    Ok(feed)
  }

  /// With cookies, a response served as signed out means the session is gone.
  fn check_session(&self, response: &Value) -> Result<()> {
    if self.session.is_some() && tracking_param(response, "logged_in").as_deref() != Some("1") {
      return Err(Error::NotLoggedIn);
    }
    Ok(())
  }

  /// A WEB client request, with the session cookies when `signed` and cookies are set.
  async fn web(&self, endpoint: &'static str, mut body: Value, signed: bool) -> Result<Value> {
    let (version, visitor) = {
      let state = self.state();
      (
        state
          .web_version
          .clone()
          .unwrap_or_else(|| WEB_CLIENT_VERSION.to_owned()),
        state.visitor_data.clone(),
      )
    };
    let (hl, gl) = &*self.language;
    body["context"] = json!({ "client": {
      "clientName": "WEB",
      "clientVersion": version,
      "hl": hl,
      "gl": gl,
    }});
    let mut request = self
      .http
      .post(format!("{ORIGIN}/youtubei/v1/{endpoint}?prettyPrint=false"))
      .header("Origin", ORIGIN)
      .header("X-Origin", ORIGIN)
      .header("X-Youtube-Client-Name", "1")
      .header("X-Youtube-Client-Version", &version)
      .json(&body);
    let session = self.session.as_ref().filter(|_| signed);
    if let (Some(visitor), true) = (&visitor, signed) {
      request = request.header("X-Goog-Visitor-Id", visitor);
    }
    if let Some(session) = session {
      let cookies = lock(&session.cookies);
      request = request
        .header("Cookie", cookies.header())
        .header("Authorization", cookies.authorization())
        .header("X-Goog-AuthUser", "0");
    }
    let response = self.send(endpoint, request, signed).await?;
    if signed && let Some(visitor) = response["responseContext"]["visitorData"].as_str() {
      self
        .state()
        .visitor_data
        .get_or_insert_with(|| visitor.to_owned());
    }
    Ok(response)
  }

  async fn send(
    &self,
    endpoint: &'static str,
    request: reqwest::RequestBuilder,
    signed: bool,
  ) -> Result<Value> {
    let response = request.send().await?;
    if signed {
      self.keep_rotated_cookies(&response);
    }
    let status = response.status();
    if !status.is_success() {
      return Err(Error::Status {
        status: status.as_u16(),
        endpoint,
      });
    }
    Ok(response.json().await?)
  }

  /// Puts back the original title of `videos`, when asked for ([`Client::with_original_titles`]).
  async fn untranslate(&self, videos: impl Iterator<Item = &mut VideoItem>) {
    let titles = videos.map(|v| (v.id.as_str(), &mut v.title)).collect();
    self.untranslate_titles(titles).await;
  }

  /// Replaces each title by its video's original one (by id). A failed lookup keeps the title
  /// as it is, and is tried again next time.
  async fn untranslate_titles(&self, titles: Vec<(&str, &mut String)>) {
    if !self.original_titles || titles.is_empty() {
      return;
    }
    let missing: Vec<String> = {
      let state = self.state();
      let mut seen = HashSet::new();
      titles
        .iter()
        .map(|(id, _)| *id)
        .filter(|id| !state.titles.contains_key(*id) && seen.insert(*id))
        .map(str::to_owned)
        .collect()
    };
    for batch in missing.chunks(TITLE_BATCH) {
      let found = join_all(batch.iter().map(|id| self.original_title(id)).collect()).await;
      let mut state = self.state();
      if state.titles.len() + batch.len() > TITLES_KEPT {
        state.titles.clear();
      }
      for (id, title) in batch.iter().zip(found) {
        match title {
          Ok(title) => {
            state.titles.insert(id.clone(), title);
          }
          Err(e) => tracing::debug!("original title of {id}: {e}"),
        }
      }
    }
    let state = self.state();
    for (id, title) in titles {
      if let Some(Some(original)) = state.titles.get(id) {
        title.clone_from(original);
      }
    }
  }

  /// A video's original title from oEmbed, anonymous: `None` when oEmbed refuses the video
  /// (401 when embedding is disabled, 403 private, 400 or 404 unknown). Other failures (429
  /// included) are errors, so the lookup is tried again later.
  async fn original_title(&self, video_id: &str) -> Result<Option<String>> {
    // The id is 11 characters of `A-Za-z0-9-_`: only the watch URL's `?` and `=` need escaping.
    let url = format!("{ORIGIN}/oembed?url={ORIGIN}/watch%3Fv%3D{video_id}&format=json");
    let response = self.http.get(url).timeout(TITLE_TIMEOUT).send().await?;
    let status = response.status();
    if matches!(status.as_u16(), 400 | 401 | 403 | 404) {
      return Ok(None);
    }
    if !status.is_success() {
      return Err(Error::Status {
        status: status.as_u16(),
        endpoint: "oembed",
      });
    }
    Ok(parse_oembed_title(&response.json().await?))
  }

  /// visitorData for VISIONOS. Signed out, the session's one is anonymous and reused.
  async fn resolver_visitor(&self) -> Result<String> {
    {
      let mut state = self.state();
      if state.resolver_visitor.is_none() && self.session.is_none() {
        state.resolver_visitor = state.visitor_data.clone();
      }
      if let Some(visitor) = state.resolver_visitor.clone() {
        return Ok(visitor);
      }
    }
    // Any anonymous request returns one; this browse is the cheapest known.
    let response = self
      .web("browse", json!({ "browseId": "FEwhat_to_watch" }), false)
      .await?;
    let visitor = response["responseContext"]["visitorData"]
      .as_str()
      .map(str::to_owned)
      .ok_or(Error::Parse {
        endpoint: "browse",
        detail: "visitorData missing".into(),
      })?;
    self.state().resolver_visitor = Some(visitor.clone());
    Ok(visitor)
  }

  /// The VISIONOS `player` answer of `video_id` (anonymous: its own visitorData).
  pub(crate) async fn visionos_player(&self, video_id: &str) -> Result<Value> {
    let visitor = self.resolver_visitor().await?;
    let (hl, gl) = &*self.language;
    let body = json!({
      "context": { "client": {
        "clientName": "VISIONOS",
        "clientVersion": VISIONOS_CLIENT_VERSION,
        "deviceMake": "Apple",
        "deviceModel": "RealityDevice17,1",
        "userAgent": VISIONOS_USER_AGENT,
        "osName": "visionOS",
        "osVersion": "26.5.23O471",
        "hl": hl,
        "gl": gl,
        "visitorData": visitor,
      }},
      "videoId": video_id,
      "contentCheckOk": true,
      "racyCheckOk": true,
    });
    let request = self
      .http
      .post(format!("{ORIGIN}/youtubei/v1/player?prettyPrint=false"))
      .header("User-Agent", VISIONOS_USER_AGENT)
      .header("Origin", ORIGIN)
      .header("X-Youtube-Client-Name", VISIONOS_CLIENT_NAME_ID)
      .header("X-Youtube-Client-Version", VISIONOS_CLIENT_VERSION)
      .header("X-Goog-Visitor-Id", &visitor)
      .json(&body);
    self.send("player", request, false).await
  }

  async fn visionos_stream(&self, video_id: &str, max_height: u32) -> Result<Stream> {
    let player = self.visionos_player(video_id).await?;
    // A live with DVR plays from its DASH segments through the local proxy: the whole broadcast
    // (ADR 0025). Should that fail, its HLS window (an hour).
    let formats = parse_live_formats(&player);
    if !formats.is_empty() {
      match LiveProxy::start(self.clone(), video_id, &player, formats, max_height).await {
        Ok(proxy) => return Ok(proxied_live(proxy, &player)),
        Err(e) => tracing::warn!("live proxy: {e}, playing the HLS window"),
      }
    }
    let master_url = hls_manifest_url(&player)?;

    let response = self
      .http
      .get(master_url)
      .header("User-Agent", VISIONOS_USER_AGENT)
      .send()
      .await?;
    if !response.status().is_success() {
      return Err(Error::Status {
        status: response.status().as_u16(),
        endpoint: "hls master",
      });
    }
    let master = response.text().await?;
    let qualities = hls::ladder(&master);
    let variant = qualities
      .iter()
      .find(|v| v.height <= max_height)
      .or(qualities.last())
      .cloned()
      .ok_or(Error::Parse {
        endpoint: "hls master",
        detail: "no variant".into(),
      })?;
    // A live's playlists are windows: mpv's timeline starts at their first segment, so playback
    // must start near their end (mpv started at 0 waits for the audio rendition, ~10 s).
    let live = if player["videoDetails"]["isLive"] == true {
      let length = self.media_playlist_length(&variant.url).await?;
      parse_live(&player, length)
    } else {
      None
    };
    Ok(Stream {
      video_url: variant.url,
      audio_url: variant.audio_url,
      resolution: variant.resolution,
      codecs: variant.codecs,
      user_agent: VISIONOS_USER_AGENT,
      captions: parse_captions(&player),
      storyboard: parse_storyboard(&player),
      qualities,
      live,
    })
  }

  /// Seconds the media playlist at `url` covers ([`hls::playlist_length`]).
  async fn media_playlist_length(&self, url: &str) -> Result<f64> {
    let response = self
      .http
      .get(url)
      .header("User-Agent", VISIONOS_USER_AGENT)
      .send()
      .await?;
    if !response.status().is_success() {
      return Err(Error::Status {
        status: response.status().as_u16(),
        endpoint: "hls media",
      });
    }
    Ok(hls::playlist_length(&response.text().await?))
  }
}

/// Runs `futures` concurrently and returns their outputs in order.
async fn join_all<F: Future>(futures: Vec<F>) -> Vec<F::Output> {
  let mut futures: Vec<_> = futures.into_iter().map(Box::pin).collect();
  let mut outputs: Vec<Option<F::Output>> = futures.iter().map(|_| None).collect();
  std::future::poll_fn(|cx| {
    let mut pending = false;
    for (future, output) in futures.iter_mut().zip(outputs.iter_mut()) {
      // A finished future is never polled again.
      if output.is_none() {
        match future.as_mut().poll(cx) {
          Poll::Ready(value) => *output = Some(value),
          Poll::Pending => pending = true,
        }
      }
    }
    if pending {
      Poll::Pending
    } else {
      Poll::Ready(())
    }
  })
  .await;
  outputs.into_iter().flatten().collect()
}

/// A poisoned lock only means a panic elsewhere; the value stays usable.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
  mutex
    .lock()
    .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A client playback nonce: 16 characters of `A-Za-z0-9-_`. YouTube accepts any (yt-dlp); this
/// one hashes the time and a counter, so two playbacks never share one.
pub(crate) fn new_cpn() -> String {
  use sha1::{Digest, Sha1};
  use std::sync::atomic::{AtomicU64, Ordering};
  const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
  static COUNTER: AtomicU64 = AtomicU64::new(0);
  let nanos = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map_or(0, |d| d.as_nanos());
  let count = COUNTER.fetch_add(1, Ordering::Relaxed);
  let digest = Sha1::digest(format!("{nanos} {count} {}", std::process::id()).as_bytes());
  digest
    .iter()
    .take(16)
    .map(|b| char::from(ALPHABET[usize::from(b & 63)]))
    .collect()
}

/// A live played through `proxy`: its qualities, the one picked at the start, and a window up
/// to YouTube's DVR length.
fn proxied_live(proxy: LiveProxy, player: &Value) -> Stream {
  let main = proxy.main().clone();
  Stream {
    video_url: main.url,
    audio_url: main.audio_url,
    resolution: main.resolution,
    codecs: main.codecs,
    user_agent: VISIONOS_USER_AGENT,
    qualities: proxy.variants().to_vec(),
    captions: parse_captions(player),
    // A live's storyboard is another kind: no seek previews.
    storyboard: None,
    live: Some(Live {
      length: proxy.length(),
      dvr_window: Some(proxy.window()),
      proxy: Some(proxy),
    }),
  }
}

/// `streamingData.hlsManifestUrl` of a player response, or why there is none.
pub fn hls_manifest_url(player: &Value) -> Result<&str> {
  let status = &player["playabilityStatus"];
  if status["status"] != "OK" {
    return Err(Error::Unplayable {
      status: status["status"].as_str().unwrap_or("?").to_owned(),
      reason: status["reason"].as_str().unwrap_or_default().to_owned(),
    });
  }
  player["streamingData"]["hlsManifestUrl"]
    .as_str()
    .ok_or(Error::Parse {
      endpoint: "player",
      detail: "no hlsManifestUrl".into(),
    })
}

/// `"KEY":"value"` in the ytcfg of a page.
fn ytcfg_string(html: &str, key: &str) -> Option<String> {
  let pattern = format!("\"{key}\":\"");
  let start = html.find(&pattern)? + pattern.len();
  let end = html[start..].find('"')?;
  Some(html[start..start + end].to_owned())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn ytcfg_value() {
    let html = r#"ytcfg.set({"INNERTUBE_CLIENT_VERSION":"2.20261029.11.00","X":1});"#;
    assert_eq!(
      ytcfg_string(html, "INNERTUBE_CLIENT_VERSION").as_deref(),
      Some("2.20261029.11.00")
    );
    assert_eq!(ytcfg_string(html, "VISITOR_DATA"), None);
  }
}
