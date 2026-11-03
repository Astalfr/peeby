use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tracing::info;

use crate::error::{Error, Result};
use crate::gql::{self, PlaybackToken, Target};
use crate::proxy::Proxy;
use crate::usher::{self, Codec, PLAYER_ORIGIN, Variant, Version};
use crate::{points, spade};

/// VODs per [`Client::vod_positions`] request: GQL allows 15 aliased root fields in a query
/// (checked 2026-11-02).
const POSITIONS_PER_REQUEST: usize = 15;

const WEB_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";

/// A master playlist as usher served it.
pub struct Master {
  /// Path version that answered.
  pub version: Version,
  /// Its text (signed variant URLs inside: never log it).
  pub text: String,
  /// Its variants, best first.
  pub variants: Vec<Variant>,
}

/// Twitch client. Cheap to clone; clones share the Client-ID.
#[derive(Clone)]
pub struct Client {
  pub(crate) http: reqwest::Client,
  /// The web Client-ID for GQL, refreshed by [`Client::refresh_client_id`].
  client_id: Arc<RwLock<String>>,
  /// The web `auth-token`, sent to GQL as `Authorization: OAuth …` (a full-access secret).
  auth_token: Option<Arc<str>>,
  /// The watch-time endpoint, read from twitch.tv once it is first needed.
  spade_url: Arc<tokio::sync::Mutex<Option<String>>>,
}

impl std::fmt::Debug for Client {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("Client")
      .field("auth_token", &self.auth_token.as_ref().map(|_| "set"))
      .finish_non_exhaustive()
  }
}

impl Client {
  /// A client with the web Client-ID `client_id` (the settings' override, else
  /// [`gql::WEB_CLIENT_ID`]) and the user's `auth_token`, if any.
  pub fn new(client_id: Option<String>, auth_token: Option<String>) -> Result<Self> {
    let http = reqwest::Client::builder()
      .user_agent(WEB_USER_AGENT)
      .build()?;
    Ok(Self {
      http,
      client_id: Arc::new(RwLock::new(
        client_id.unwrap_or_else(|| gql::WEB_CLIENT_ID.to_owned()),
      )),
      auth_token: auth_token.map(Arc::from),
      spade_url: Arc::default(),
    })
  }

  fn client_id(&self) -> String {
    // A poisoned lock only means a writer panicked mid-assignment of a `String`: the value is
    // still a whole string, so it is used as is.
    match self.client_id.read() {
      Ok(id) => id.clone(),
      Err(poisoned) => poisoned.into_inner().clone(),
    }
  }

  /// Reads the web Client-ID from twitch.tv's page, in case Twitch changed it. Call it once at
  /// startup, in the background; on failure the current one stays.
  pub async fn refresh_client_id(&self) -> Result<()> {
    let page = self
      .http
      .get("https://www.twitch.tv/")
      .send()
      .await?
      .error_for_status()?
      .text()
      .await?;
    let id = gql::client_id_from_page(&page).ok_or_else(|| Error::Parse {
      endpoint: "twitch.tv",
      detail: "no clientId in the page".into(),
    })?;
    if id != self.client_id() {
      tracing::info!("Twitch web Client-ID changed to {id}");
      match self.client_id.write() {
        Ok(mut current) => *current = id.to_owned(),
        Err(poisoned) => *poisoned.into_inner() = id.to_owned(),
      }
    }
    Ok(())
  }

  /// The answer to a GQL request (a `gql::*_request`), with the auth-token when there is one;
  /// public for the fixtures' capture.
  pub async fn gql(&self, body: &Value) -> Result<Value> {
    self.gql_with_client_id(body, &self.client_id()).await
  }

  /// The answer to a GQL request (a `gql::*_request`), with the auth-token when there is one;
  /// public for the fixtures' capture.
  pub async fn gql_with_client_id(&self, body: &Value, client_id: &str) -> Result<Value> {
    let mut request = self
      .http
      .post("https://gql.twitch.tv/gql")
      .header("Client-ID", client_id)
      .json(body);
    if let Some(token) = &self.auth_token {
      request = request.header("Authorization", format!("OAuth {token}"));
    }
    let response = request.send().await?;
    let status = response.status().as_u16();
    let text = response.text().await?;
    if status != 200 {
      return Err(gql::parse_status(status, &text));
    }
    serde_json::from_str(&text).map_err(|e| Error::Parse {
      endpoint: "gql",
      detail: e.to_string(),
    })
  }

  /// `login`'s channel and its live, if on air.
  pub async fn channel(&self, login: &str) -> Result<gql::Channel> {
    let answer = self.gql(&gql::channel_request(login)).await?;
    gql::parse_channel(&answer)
  }

  /// A clip's files (direct MP4s, signed).
  pub async fn clip(&self, slug: &str) -> Result<gql::ClipPlayback> {
    let answer = self.gql(&gql::clip_request(slug)).await?;
    gql::parse_clip_playback(&answer)
  }

  /// A VOD's storyboard levels (largest frames first); empty when it has none.
  pub async fn storyboard(&self, id: &str) -> Result<Vec<gql::StoryboardLevel>> {
    let answer = self.gql(&gql::seek_previews_request(id)).await?;
    let Some(url) = gql::parse_seek_previews(&answer) else {
      return Ok(Vec::new());
    };
    let response = self.http.get(&url).send().await?;
    if !response.status().is_success() {
      return Ok(Vec::new());
    }
    Ok(gql::parse_storyboard(&response.text().await?, &url))
  }

  /// VOD `id`'s game chapters, in order; empty when it kept one game.
  pub async fn vod_chapters(&self, id: &str) -> Result<Vec<gql::Chapter>> {
    let answer = self.gql(&gql::chapters_request(id)).await?;
    gql::parse_chapters(&answer)
  }

  /// Where the viewer stopped in VOD `id` (their account's, as twitch.tv resumes it). Without an
  /// auth-token, no position and no user id.
  pub async fn vod_viewing(&self, id: &str) -> Result<gql::Viewing> {
    let answer = self.gql(&gql::viewing_request(id)).await?;
    gql::parse_viewing(&answer)
  }

  /// Where the viewer stopped in each of the VODs `ids`, by id (for the cards' progress bars),
  /// a request per [`POSITIONS_PER_REQUEST`]; the ones never watched are left out. Without an
  /// auth-token, nothing.
  pub async fn vod_positions(&self, ids: &[String]) -> Result<HashMap<String, u32>> {
    let mut positions = HashMap::new();
    if self.auth_token.is_none() {
      return Ok(positions);
    }
    for chunk in ids.chunks(POSITIONS_PER_REQUEST) {
      let answer = self.gql(&gql::positions_request(chunk)).await?;
      positions.extend(gql::parse_positions(&answer)?);
    }
    Ok(positions)
  }

  /// Saves `position` seconds as `user_id`'s resume position in VOD `id`, as twitch.tv's player
  /// does (needs the auth-token).
  pub async fn save_vod_position(&self, user_id: &str, id: &str, position: u32) -> Result<()> {
    if self.auth_token.is_none() {
      return Err(Error::NotLoggedIn);
    }
    let answer = self
      .gql(&gql::viewed_request(user_id, id, position))
      .await?;
    gql::parse_viewed(&answer)
  }

  /// A page of VOD `id`'s chat from about `offset` seconds.
  pub async fn vod_comments(&self, id: &str, offset: u32) -> Result<gql::Comments> {
    let answer = self.gql(&gql::comments_request(id, offset)).await?;
    gql::parse_comments(&answer)
  }

  /// The 7TV emotes of the channel whose Twitch user id is `user_id`; empty without a 7TV account.
  pub async fn seventv_channel(&self, user_id: &str) -> Result<Vec<crate::seventv::Emote>> {
    crate::seventv::channel(&self.http, user_id).await
  }

  /// 7TV's global emotes.
  pub async fn seventv_global(&self) -> Result<Vec<crate::seventv::Emote>> {
    crate::seventv::global(&self.http).await
  }

  /// The BTTV emotes of the channel whose Twitch user id is `user_id`; empty without a BTTV account.
  pub async fn bttv_channel(&self, user_id: &str) -> Result<Vec<crate::bttv::Emote>> {
    crate::bttv::channel(&self.http, user_id).await
  }

  /// BTTV's global emotes.
  pub async fn bttv_global(&self) -> Result<Vec<crate::bttv::Emote>> {
    crate::bttv::global(&self.http).await
  }

  /// Who the client's auth-token belongs to (its login): checks a pasted token before it is saved.
  /// [`Error::NotLoggedIn`] when it is refused or there is none.
  pub async fn current_user(&self) -> Result<String> {
    let answer = self.gql(&gql::current_user_request()).await?;
    gql::parse_current_user(&answer)
  }

  /// The viewer's channel points on `login`'s channel and its rewards (needs the auth-token).
  pub async fn points(&self, login: &str) -> Result<points::Points> {
    let answer = self.gql(&points::context_request(login)).await?;
    points::parse_context(&answer)
  }

  /// `login`'s open and locked predictions.
  pub async fn predictions(&self, login: &str) -> Result<Vec<points::Prediction>> {
    let answer = self.gql(&points::predictions_request(login)).await?;
    points::parse_predictions(&answer)
  }

  /// Redeems `reward` on `channel_id`: spends the viewer's points.
  pub async fn redeem(
    &self,
    channel_id: &str,
    reward: &points::Reward,
    text: Option<&str>,
    transaction_id: &str,
  ) -> Result<()> {
    let request = points::redeem_request(channel_id, reward, text, transaction_id);
    points::parse_redeem(&self.gql(&request).await?)
  }

  /// Bets `points` on `outcome_id` of the prediction `event_id`: spends the viewer's points.
  pub async fn predict(
    &self,
    event_id: &str,
    outcome_id: &str,
    points: u64,
    transaction_id: &str,
  ) -> Result<()> {
    let request = points::predict_request(event_id, outcome_id, points, transaction_id);
    points::parse_predict(&self.gql(&request).await?)
  }

  /// Reports a minute of `event`'s live watched (Twitch's `minute-watched`), which earns its
  /// channel points. Send it once a minute, only while the live plays on screen.
  pub async fn minute_watched(&self, event: &spade::MinuteWatched) -> Result<()> {
    let url = self.spade_url().await?;
    let response = self
      .http
      .post(&url)
      .header("Content-Type", "application/x-www-form-urlencoded")
      .body(spade::body(event))
      .send()
      .await?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
      return Err(Error::Status {
        status,
        endpoint: "spade",
        code: None,
      });
    }
    Ok(())
  }

  /// `spade_url` from twitch.tv's settings script, kept once read (a failure is tried again
  /// next time).
  async fn spade_url(&self) -> Result<String> {
    let mut cached = self.spade_url.lock().await;
    if let Some(url) = cached.as_ref() {
      return Ok(url.clone());
    }
    let missing = |detail: &str| Error::Parse {
      endpoint: "twitch.tv",
      detail: detail.into(),
    };
    let page = self
      .http
      .get("https://www.twitch.tv/")
      .send()
      .await?
      .error_for_status()?
      .text()
      .await?;
    let settings = spade::settings_url(&page).ok_or_else(|| missing("no settings script"))?;
    let script = self
      .http
      .get(settings)
      .send()
      .await?
      .error_for_status()?
      .text()
      .await?;
    let url = spade::spade_url(&script).ok_or_else(|| missing("no spade_url"))?;
    *cached = Some(url.clone());
    Ok(url)
  }

  /// The playback token of `target`, as `player_type` (normally [`gql::PLAYER_TYPE`]).
  pub async fn playback_token(&self, target: &Target, player_type: &str) -> Result<PlaybackToken> {
    let answer = self.gql(&gql::token_request(target, player_type)).await?;
    gql::parse_token(&answer, target)
  }

  /// The master playlist of `target` with `token`, offering `codecs`. usher v2 first, then v1
  /// when v2 fails for another reason than one of the mapped errors (offline, subscribers…).
  pub async fn master(
    &self,
    target: &Target,
    token: &PlaybackToken,
    codecs: &[Codec],
  ) -> Result<Master> {
    match self.master_at(target, token, codecs, Version::V2).await {
      Err(Error::Status { .. } | Error::Http(_)) => {
        tracing::warn!("usher v2 failed, trying v1");
        self.master_at(target, token, codecs, Version::V1).await
      }
      result => result,
    }
  }

  async fn master_at(
    &self,
    target: &Target,
    token: &PlaybackToken,
    codecs: &[Codec],
    version: Version,
  ) -> Result<Master> {
    info!("usher {version:?} master for {target:?} with codecs {codecs:?} and token {token:?}");
    let url = usher::master_url(target, token, codecs, version, nonce())?;
    let response = self
      .http
      .get(url)
      .header("Referer", PLAYER_ORIGIN)
      .header("Origin", PLAYER_ORIGIN)
      .send()
      .await?;
    let status = response.status().as_u16();
    let text = response.text().await?;
    if status != 200 {
      let error = usher::parse_error(status, &text, target);
      if let Ok(master) = crate::fallback::fallback(&error, self, target, version).await {
        return Ok(master);
      }
      return Err(error);
    }
    let variants = usher::variants(&text);
    if variants.is_empty() {
      return Err(Error::Parse {
        endpoint: "usher",
        detail: "no variant in the master".into(),
      });
    }
    info!(
      "usher {version:?} master for {target:?} variants: {:?} text: {}",
      variants, text
    );
    Ok(Master {
      version,
      text,
      variants,
    })
  }
}

impl Client {
  /// Everything to play `target` through the local proxy. For a live, the main token
  /// ([`gql::PLAYER_TYPE`]) and a backup one ([`gql::BACKUP_PLAYER_TYPE`], to fill ad breaks) are
  /// asked for together, then both masters; a failing backup only costs the ad cover. A live gets
  /// Twitch's prefetch segments when `low_latency`. Call it on a tokio runtime.
  pub async fn play(&self, target: &Target, codecs: &[Codec], low_latency: bool) -> Result<Proxy> {
    self
      .play_with(
        target,
        codecs,
        gql::PLAYER_TYPE,
        gql::BACKUP_PLAYER_TYPE,
        low_latency,
      )
      .await
  }

  /// [`Client::play`] with other `playerType`s: `site` as the main one gets the pre-roll, which
  /// shows the backup filling it (spike, tests by hand).
  pub async fn play_with(
    &self,
    target: &Target,
    codecs: &[Codec],
    main_type: &str,
    backup_type: &str,
    low_latency: bool,
  ) -> Result<Proxy> {
    let live = matches!(target, Target::Live(_));
    let backup = async {
      if !live {
        return None;
      }
      let token = self
        .playback_token(target, backup_type)
        .await
        .inspect_err(|e| tracing::warn!("Twitch backup token: {e}"))
        .ok()?;
      self
        .master(target, &token, codecs)
        .await
        .inspect_err(|e| tracing::warn!("Twitch backup master: {e}"))
        .ok()
    };
    let main = async {
      let token = self.playback_token(target, main_type).await?;
      self.master(target, &token, codecs).await
    };
    let (main, backup) = tokio::join!(main, backup);
    Proxy::start(self.clone(), target, main?, backup, low_latency).await
  }

  /// A media playlist (live or VOD), with the player's `Referer` and `Origin`.
  pub(crate) async fn media_playlist(&self, url: &str) -> Result<String> {
    let response = self
      .http
      .get(url)
      .header("Referer", PLAYER_ORIGIN)
      .header("Origin", PLAYER_ORIGIN)
      .send()
      .await?;
    let status = response.status().as_u16();
    if status != 200 {
      return Err(Error::Status {
        status,
        endpoint: "playlist",
        code: None,
      });
    }
    Ok(response.text().await?)
  }
}

/// usher's `p`: any number, different per request.
fn nonce() -> u32 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map_or(0, |d| d.subsec_nanos() % 10_000_000)
}
