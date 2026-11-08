//! What plays in the Twitch space, with its header under the player:
//! - a channel's live: the shared player in live mode through the local proxy, the channel's
//!   header (GQL), and its chat beside it (`ui::chat`, fed by [`super::chat`]). While an ad break runs that the backup
//!   stream cannot cover, the player shows a notice with its countdown; the viewers are refreshed
//!   every minute; the viewer writes in the chat with the auth-token, their channel points under
//!   its field and the predictions and polls pinned over its messages ([`super::points`]);
//!   The live's past plays from its recording (its archive VOD, through a second proxy) when the
//!   channel keeps its lives: the seek bar covers the broadcast (ADR 0026);
//! - a VOD: through the proxy too, seekable, with the storyboard's previews, its muted parts and
//!   its game chapters on the seek bar, and its chat replayed beside it as the playhead moves ([`super::replay`]);
//! - a clip: its direct MP4s.
//!
//! The layout is `ui::chat::watch_layout`: theater mode gives the whole page to the player and the
//! chat, flush (`Settings::theater`, shared). The channel's name opens its page.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui::prelude::*;
use gpui::{Context, Entity, FontWeight, SharedString, Subscription, Task, Window, div, px};
use gpui_component::{ActiveTheme, h_flex, v_flex};
use twitch::catalog::Catalog;
use twitch::gql::Channel;
use twitch::usher::Codec;
use twitch::{Proxy, Target};
use ui::chat::{ChatPanel, ChatPanelEvent, watch_layout};
use ui::i18n::t;
use ui::images::Images;
use ui::picture::{self, Picture};
use ui::player::{Author, LiveInfo, PlayerEvent, PlayerView, Rewind, Source};
use ui::{runtime, tf, theme};

use super::points::{Backend, PointsEvent, PointsFooter, PointsView};
use super::{Nav, Route, cards, chat, follow, play, progress, replay};
use peeby_core::settings::{Player, Settings};
use peeby_core::shell::space::middle_click;

/// How often the decoding of an AV1 or HEVC quality is checked.
const DECODING_EVERY: Duration = Duration::from_secs(4);
/// How often a live's viewers and category are read again.
const INFO_EVERY: Duration = Duration::from_secs(60);

/// What the page plays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
  /// A channel's live, by login.
  Live(String),
  /// A VOD, by id, from `start` seconds instead of the viewer's resume position.
  Vod { id: String, start: Option<u32> },
  /// A clip, by slug.
  Clip(String),
}

/// The header under the player.
#[derive(Clone, Default)]
struct Header {
  title: String,
  /// The channel's shown name and login (its page on click).
  name: String,
  login: String,
  avatar: Option<String>,
  /// Category, viewers, uptime (a live); views and date (a VOD, a clip).
  line: Option<String>,
  /// Whether the account follows the channel; `None` without an auth-token.
  followed: Option<bool>,
}

impl Header {
  /// The channel shown over the player with the title.
  fn author(&self) -> Option<Author> {
    if self.name.is_empty() {
      return None;
    }
    Some(Author {
      name: self.name.clone().into(),
      avatar: self
        .avatar
        .clone()
        .map(|url| Picture::sized(move |_, _| url.clone())),
    })
  }
}

/// Lets `panel`'s user cards find a user's picture in `catalog`, and open their channel in a
/// new tab of `nav`'s, the live going on.
fn user_cards(panel: &Entity<ChatPanel>, catalog: &Catalog, nav: &Nav, cx: &mut gpui::App) {
  let lookup = chat::user_lookup(catalog.clone());
  panel.update(cx, |panel, _| panel.set_user_lookup(lookup));
  let nav = nav.clone();
  cx.subscribe(panel, move |_, event: &ChatPanelEvent, cx| {
    if let ChatPanelEvent::OpenChannel { channel, name } = event {
      nav.open_in_new_tab(Route::Channel(channel.to_string()), Some(name.clone()), cx);
    }
  })
  .detach();
}

/// The chat's header closes it in theater mode: kept in the settings, as theater mode.
fn hide_on_request(panel: &Entity<ChatPanel>, cx: &mut Context<WatchView>) {
  cx.subscribe(panel, |_, _, event: &ChatPanelEvent, cx| {
    if let ChatPanelEvent::Hide = event {
      Settings::update(cx, |s| s.chat_hidden = true);
      cx.notify();
    }
  })
  .detach();
}

pub struct WatchView {
  kind: Kind,
  client: twitch::Client,
  catalog: Catalog,
  nav: Nav,
  player: Entity<PlayerView>,
  /// Serves mpv's playlists (a live, a VOD); dropping it (with the page) stops it.
  proxy: Option<Proxy>,
  /// A live's chat, or a VOD's replayed.
  chat: Option<Entity<ChatPanel>>,
  /// A live's channel points (shows nothing without the auth-token).
  points: Option<Entity<PointsView>>,
  /// Sends what the viewer writes in a live's chat (with the auth-token).
  _chat_send: Option<Subscription>,
  /// Does what the viewer chooses under a live's chat gear (reloading the 7TV emotes).
  _chat_options: Option<Subscription>,
  /// The codec and height of each quality of a live or a VOD, for the decoding check.
  qualities: play::QualityCodecs,
  /// The quality whose last check read `hwdec-current` as `no`: a second reading in a row acts.
  cpu_reading: Option<usize>,
  header: Option<Header>,
  /// A live's start, for the header's uptime.
  started_at: Option<i64>,
  /// Saves a VOD's position to the account (with the auth-token).
  saver: Option<progress::Saver>,
  /// A live's recording, as its channel names it, until its playback is asked for.
  archive: Option<twitch::gql::Archive>,
  /// Whether the recording was asked for (once per page).
  rewind_asked: bool,
  /// Serves the recording's playlists.
  archive_proxy: Option<Proxy>,
  /// Saves the position in the recording while it plays, as for any VOD (ADR 0013).
  archive_saver: Option<progress::Saver>,
  focused: bool,
  _tasks: Vec<Task<()>>,
}

impl WatchView {
  pub fn new(
    kind: Kind,
    client: twitch::Client,
    catalog: Catalog,
    nav: Nav,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let settings = Settings::get(cx);
    let (prefs, theater) = (Player::prefs(settings), settings.theater);
    let player = cx.new(|cx| PlayerView::new(prefs, theater, cx));
    let live = matches!(kind, Kind::Live(_));
    cx.subscribe(&player, move |this, player, event: &PlayerEvent, cx| {
      if let Some(saver) = &mut this.saver {
        saver.on_event(event, &player, cx);
      }
      if let Some(saver) = &mut this.archive_saver {
        match *event {
          // Leaving the recording for the live: where it was left.
          PlayerEvent::Rewound { rewound: false, at } => saver.save_at(at, cx),
          _ if player.read(cx).rewound() => saver.on_event(event, &player, cx),
          _ => {}
        }
      }
      match *event {
        PlayerEvent::Theater => {
          Settings::update(cx, |s| s.theater = !s.theater);
          let theater = Settings::get(cx).theater;
          player.update(cx, |player, cx| player.set_theater(theater, cx));
          cx.notify();
        }
        PlayerEvent::Sound { volume, muted } => Player::save_sound(volume, muted, cx),
        PlayerEvent::Ended if live => {
          player.update(cx, |player, cx| {
            player.set_notice(
              Some(t("The live has ended.", "Le live est terminé.").into()),
              cx,
            )
          });
        }
        _ => {}
      }
    })
    .detach();
    // The page goes: the VOD's last position.
    cx.on_release(|this: &mut Self, cx| {
      let player = this.player.clone();
      if let Some(saver) = &mut this.saver {
        saver.save(&player, cx);
      }
      if let Some(saver) = &mut this.archive_saver
        && player.read(cx).rewound()
      {
        saver.save(&player, cx);
      }
    })
    .detach();
    let mut view = Self {
      kind,
      client,
      catalog,
      nav,
      player,
      proxy: None,
      chat: None,
      points: None,
      _chat_send: None,
      _chat_options: None,
      qualities: Vec::new(),
      cpu_reading: None,
      header: None,
      started_at: None,
      saver: None,
      archive: None,
      rewind_asked: false,
      archive_proxy: None,
      archive_saver: None,
      focused: false,
      _tasks: Vec::new(),
    };
    match view.kind.clone() {
      Kind::Live(login) => {
        let panel = cx.new(|cx| ChatPanel::new(cx).with_composer(twitch::chat::MAX_CHARS));
        user_cards(&panel, &view.catalog, &view.nav, cx);
        // The streamer's mentions in red, those of the signed-in viewer on a red tint.
        let nav = view.nav.clone();
        let viewer: ui::chat::Viewer = std::rc::Rc::new(move |cx: &gpui::App| {
          nav
            .upgrade()
            .and_then(|nav| nav.read(cx).account().map(|a| a.login.clone().into()))
        });
        panel.update(cx, |panel, _| {
          panel.set_streamer(login.clone());
          panel.set_viewer(viewer);
        });
        let (reader, extras) = chat::start(
          &login,
          view.client.clone(),
          view.catalog.clone(),
          &panel,
          cx,
        );
        view._tasks.push(reader);
        view._chat_options = Some(chat::start_options(
          extras,
          view.client.clone(),
          view.catalog.clone(),
          &panel,
          cx,
        ));
        // Read once the writer starts: the page is built while the space is being updated.
        let nav = view.nav.clone();
        let account = move |cx: &mut gpui::App| {
          nav
            .upgrade()
            .and_then(|nav| nav.read(cx).account().map(|a| a.login.clone()))
        };
        if let Some((task, send)) =
          chat::start_writer(&login, view.client.clone(), account, &panel, cx)
        {
          view._tasks.push(task);
          view._chat_send = Some(send);
        }
        let client = view.client.clone();
        let login_ = login.clone();
        let player = view.player.downgrade();
        let points = cx.new(|cx| PointsView::new(login_, Backend::Twitch(client), player, cx));
        // The balance and its rewards under the chat's field, the predictions and polls over
        // its messages.
        // The raid the viewer follows opens the raided channel's live, after this event.
        cx.subscribe(&points, |this, _, event: &PointsEvent, cx| {
          let PointsEvent::Raid(login) = event;
          let (nav, route) = (this.nav.clone(), Route::Watch(login.clone()));
          cx.defer(move |cx| nav.open(route, cx));
        })
        .detach();
        let footer = cx.new(|cx| PointsFooter::new(points.clone(), cx));
        panel.update(cx, |p, cx| {
          p.set_footer(footer.into(), cx);
          p.set_pinned(points.clone().into(), cx);
        });
        hide_on_request(&panel, cx);
        view.chat = Some(panel);
        view.points = Some(points);
        view.start_live(login, cx)
      }
      Kind::Vod { id, start } => {
        let panel = cx.new(|cx| ChatPanel::new(cx).titled(t("Chat replay", "Rediffusion du chat")));
        user_cards(&panel, &view.catalog, &view.nav, cx);
        view._tasks.push(replay::start(
          id.clone(),
          view.client.clone(),
          view.catalog.clone(),
          &panel,
          view.player.downgrade(),
          cx,
        ));
        hide_on_request(&panel, cx);
        view.chat = Some(panel);
        view.start_vod(id, start, cx)
      }
      Kind::Clip(slug) => view.start_clip(slug, cx),
    }
    view
  }

  /// The player, for the shell's mini-player.
  pub fn player(&self) -> Entity<PlayerView> {
    self.player.clone()
  }

  fn max_height(cx: &Context<Self>) -> u32 {
    Settings::get(cx)
      .twitch
      .max_height
      .unwrap_or(play::DEFAULT_HEIGHT)
  }

  fn fail(&mut self, error: &twitch::Error, cx: &mut Context<Self>) {
    let name = self
      .header
      .as_ref()
      .map_or_else(|| self.login_hint(), |h| h.name.clone());
    tracing::warn!("Twitch playback: {error}");
    let mut message = play::describe(error, &name);
    let token = self
      .nav
      .upgrade()
      .is_some_and(|nav| nav.read(cx).has_auth_token());
    if matches!(error, twitch::Error::SubscriberOnly) && !token {
      message.push_str(t(
        " Subscribed? Add the Twitch site token in the settings to watch it.",
        " Abonné ? Ajoutez le jeton du site Twitch dans les réglages pour la regarder.",
      ));
    }
    self.player.update(cx, |p, cx| p.fail(message.into(), cx));
  }

  fn login_hint(&self) -> String {
    match &self.kind {
      Kind::Live(login) => login.clone(),
      _ => t("This channel", "Cette chaîne").into(),
    }
  }

  fn start_source(&mut self, source: Source, clicked: Instant, cx: &mut Context<Self>) {
    self
      .player
      .update(cx, |player, cx| player.start(source, clicked, cx));
  }

  /// The playback (tokens, masters, proxy) and the channel's header, asked for together.
  fn start_live(&mut self, login: String, cx: &mut Context<Self>) {
    let clicked = Instant::now();
    let client = self.client.clone();
    let low_latency = Settings::get(cx).twitch.low_latency.unwrap_or(true);
    let catch_up = low_latency && Settings::get(cx).twitch.catch_up.unwrap_or(true);
    let codecs = play::codecs(&Settings::get(cx).twitch.software_codecs);
    let play = runtime::spawn(cx, {
      let (client, login) = (client.clone(), login.clone());
      async move {
        client
          .play(&Target::Live(login), &codecs, low_latency)
          .await
      }
    });
    let archive = runtime::spawn(cx, {
      let (client, login) = (client.clone(), login.clone());
      async move { client.live_archive(&login).await }
    });
    let channel = runtime::spawn(cx, async move { client.channel(&login).await });
    self._tasks.push(cx.spawn(async move |this, cx| {
      let (play, channel, archive) = (play.await, channel.await, archive.await);
      this
        .update(cx, |this, cx| {
          if let Some(Ok(channel)) = channel {
            this.show_channel(channel, cx);
          }
          this.keep_archive(archive);
          match play {
            Some(Ok(proxy)) => {
              let (mut source, qualities) = play::proxied(
                &proxy,
                true,
                Self::max_height(cx),
                &Settings::get(cx).player,
              );
              // Low latency: closer to the edge than where mpv starts, and back there after a stall.
              source.catch_up = catch_up.then_some(play::CATCH_UP_BUFFER);
              this.proxy = Some(proxy);
              this.qualities = qualities;
              this.start_source(source, clicked, cx);
              this.watch_live(cx);
              this.watch_decoding(cx);
              this.prepare_rewind(cx);
              if this.archive.is_none() {
                // Asked again every minute: a live just started may have none yet.
                tracing::info!("Twitch: no recording of this live (yet): no past");
              }
            }
            Some(Err(e)) => this.fail(&e, cx),
            None => {}
          }
        })
        .ok();
    }));
  }

  /// The answer to [`twitch::Client::live_archive`], kept for [`Self::prepare_rewind`].
  fn keep_archive(&mut self, answer: Option<twitch::Result<Option<twitch::gql::Archive>>>) {
    match answer {
      Some(Ok(Some(archive))) => self.archive = Some(archive),
      Some(Ok(None)) | None => {}
      Some(Err(e)) => tracing::warn!("Twitch: the live's recording: {e}"),
    }
  }

  /// A live's past (ADR 0026): once the live plays and its recording is known, the
  /// recording's VOD through a second proxy and the viewer's position in it, then the player's
  /// [`Rewind`]. A recording Twitch refuses (subscribers only) leaves the live without its past.
  fn prepare_rewind(&mut self, cx: &mut Context<Self>) {
    if self.rewind_asked || self.proxy.is_none() {
      return;
    }
    let Some(archive) = self.archive.clone() else {
      return;
    };
    let Some(started_at) = archive
      .created_at
      .and_then(|ms| u64::try_from(ms).ok())
      .map(|ms| UNIX_EPOCH + Duration::from_millis(ms))
    else {
      tracing::info!("Twitch: the live's recording has no start time: no past");
      self.rewind_asked = true;
      return;
    };
    self.rewind_asked = true;
    let codecs = play::codecs(&Settings::get(cx).twitch.software_codecs);
    let play = runtime::spawn(cx, {
      let (client, id) = (self.client.clone(), archive.id.clone());
      async move { client.play(&Target::Vod(id), &codecs, false).await }
    });
    let viewing = runtime::spawn(cx, {
      let (client, id) = (self.client.clone(), archive.id.clone());
      async move { client.vod_viewing(&id).await }
    });
    self._tasks.push(cx.spawn(async move |this, cx| {
      let (play, viewing) = (play.await, viewing.await);
      this
        .update(cx, |this, cx| {
          let proxy = match play {
            Some(Ok(proxy)) => proxy,
            Some(Err(e)) => {
              tracing::info!("Twitch: the live's recording cannot play ({e}): no past");
              return;
            }
            None => return,
          };
          let (recording, _) = play::proxied(
            &proxy,
            false,
            Self::max_height(cx),
            &Settings::get(cx).player,
          );
          match viewing {
            Some(Ok(viewing)) => {
              this.archive_saver = progress::Saver::new(this.client.clone(), archive.id, &viewing);
              if this.archive_saver.is_some() {
                this.save_periodically(cx);
              }
            }
            Some(Err(e)) => tracing::warn!("recording position: {e}"),
            None => {}
          }
          this.archive_proxy = Some(proxy);
          tracing::info!("Twitch: the live's past plays from its recording");
          this.player.update(cx, |player, cx| {
            player.set_rewind(
              Rewind {
                started_at,
                recording,
              },
              cx,
            )
          });
        })
        .ok();
    }));
  }

  /// A live's notice and its viewers, kept up to date.
  fn watch_live(&mut self, cx: &mut Context<Self>) {
    self._tasks.push(cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor().timer(Duration::from_secs(1)).await;
        if this.update(cx, |this, cx| this.show_ad_notice(cx)).is_err() {
          return;
        }
      }
    }));
    self._tasks.push(cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor().timer(INFO_EVERY).await;
        let Ok((fetch, archive)) = this.update(cx, |this, cx| {
          let client = this.client.clone();
          let login = this.login_hint();
          // The recording until there is one (a live just started may have none yet).
          let archive = (!this.rewind_asked).then(|| {
            let (client, login) = (client.clone(), login.clone());
            runtime::spawn(cx, async move { client.live_archive(&login).await })
          });
          let fetch = runtime::spawn(cx, async move { client.channel(&login).await });
          (fetch, archive)
        }) else {
          return;
        };
        if let Some(archive) = archive {
          let answer = archive.await;
          let alive = this.update(cx, |this, cx| {
            this.keep_archive(answer);
            this.prepare_rewind(cx);
          });
          if alive.is_err() {
            return;
          }
        }
        if let Some(Ok(channel)) = fetch.await
          && this
            .update(cx, |this, cx| this.show_channel(channel, cx))
            .is_err()
        {
          return;
        }
      }
    }));
  }

  /// Checks every few seconds that the GPU decodes the quality playing when it is AV1 or HEVC:
  /// if mpv decodes it on the CPU (`hwdec-current` is `no` twice in a row, not a reading taken while
  /// a quality loads), the codec is no longer asked for (`[twitch] software_codecs`) and playback moves
  /// to the best H.264 quality not taller. Not with hardware acceleration off: mpv decodes everything
  /// on the CPU then.
  fn watch_decoding(&mut self, cx: &mut Context<Self>) {
    self._tasks.push(cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor().timer(DECODING_EVERY).await;
        let alive = this.update(cx, |this, cx| this.check_decoding(cx));
        if alive.is_err() {
          return;
        }
      }
    }));
  }

  fn check_decoding(&mut self, cx: &mut Context<Self>) {
    if !Settings::get(cx).player.hardware_acceleration
      || std::env::var("PEEBY_SOFTWARE_VIDEO").as_deref() == Ok("1")
    {
      return;
    }
    let player = self.player.read(cx);
    let Some(index) = player.quality() else {
      return;
    };
    let Some(codec @ (Codec::Av1 | Codec::H265)) = self.qualities.get(index).and_then(|q| q.0)
    else {
      self.cpu_reading = None;
      return;
    };
    if player.property("hwdec-current").as_deref() != Some("no") {
      self.cpu_reading = None;
      return;
    }
    if self.cpu_reading.replace(index) != Some(index) {
      return;
    }
    self.cpu_reading = None;
    let key = play::codec_key(codec);
    tracing::info!("Twitch: {key} decoded on the CPU, falling back to H.264");
    Settings::update(cx, |s| {
      if !s.twitch.software_codecs.iter().any(|c| c == key) {
        s.twitch.software_codecs.push(key.to_owned());
      }
    });
    if let Some(fallback) = play::fallback(&self.qualities, index) {
      self
        .player
        .update(cx, |player, cx| player.set_quality(fallback, cx));
    }
  }

  /// The VOD through the proxy, its storyboard, its game chapters, and its header.
  fn start_vod(&mut self, id: String, start: Option<u32>, cx: &mut Context<Self>) {
    let clicked = Instant::now();
    let (client, catalog) = (self.client.clone(), self.catalog.clone());
    self.load_chapters(id.clone(), cx);
    let codecs = play::codecs(&Settings::get(cx).twitch.software_codecs);
    let play = runtime::spawn(cx, {
      let (client, id) = (client.clone(), id.clone());
      async move { client.play(&Target::Vod(id), &codecs, false).await }
    });
    let storyboard = runtime::spawn(cx, {
      let (client, id) = (client.clone(), id.clone());
      async move { client.storyboard(&id).await }
    });
    // About 30 ms, asked with the playback: the start position is known when mpv starts.
    let viewing = runtime::spawn(cx, {
      let (client, id) = (client.clone(), id.clone());
      async move { client.vod_viewing(&id).await }
    });
    let video_id = id.clone();
    let info = runtime::spawn(cx, async move {
      let video = catalog.video(&id).await?;
      let avatar = cards::avatars(&catalog, vec![video.user_id.clone()]).await;
      Ok::<_, twitch::Error>((video, avatar))
    });
    self._tasks.push(cx.spawn(async move |this, cx| {
      let (play, storyboard, info) = (play.await, storyboard.await, info.await);
      let viewing = match viewing.await {
        Some(Ok(viewing)) => Some(viewing),
        Some(Err(e)) => {
          tracing::warn!("VOD position: {e}");
          None
        }
        None => None,
      };
      this
        .update(cx, |this, cx| {
          if let Some(Ok((video, avatars))) = info {
            // The replay's mentions of the streamer in red.
            if let Some(chat) = &this.chat {
              let login = video.user_login.clone();
              chat.update(cx, |chat, cx| {
                chat.set_streamer(login);
                cx.notify();
              });
            }
            this.header = Some(Header {
              title: video.title.clone(),
              name: video.user_name.clone(),
              login: video.user_login.clone(),
              avatar: avatars.get(&video.user_id).cloned(),
              line: Some(format!(
                "{} · {}",
                cards::views(video.view_count),
                cards::date(video.created_at)
              )),
              followed: None,
            });
            this.load_followed(cx);
            let title: SharedString = video.title.into();
            this.nav.set_title(cx.entity_id(), title.clone(), cx);
            let muted = play::muted(&video.muted_segments);
            let author = this.header.as_ref().and_then(Header::author);
            this.player.update(cx, |p, cx| {
              p.set_title(title, cx);
              p.set_author(author, cx);
              p.set_segments(muted, cx);
            });
          }
          match play {
            Some(Ok(proxy)) => {
              let (mut source, qualities) = play::proxied(
                &proxy,
                false,
                Self::max_height(cx),
                &Settings::get(cx).player,
              );
              source.previews = storyboard.and_then(Result::ok).and_then(play::previews);
              // A chapter or a link's time wins over where the viewer stopped.
              if let Some(start) = start {
                source.options.start_seconds = f64::from(start);
              }
              if let Some(viewing) = &viewing {
                if let Some(start) = progress::start_at(viewing).filter(|_| start.is_none()) {
                  source.options.start_seconds = start;
                }
                this.saver = progress::Saver::new(this.client.clone(), video_id, viewing);
                if this.saver.is_some() {
                  this.save_periodically(cx);
                }
              }
              this.proxy = Some(proxy);
              this.qualities = qualities;
              this.start_source(source, clicked, cx);
              this.watch_decoding(cx);
            }
            Some(Err(e)) => this.fail(&e, cx),
            None => {}
          }
          cx.notify();
        })
        .ok();
    }));
  }

  /// The VOD's game chapters, given to the player when they come: playback does not wait for them.
  fn load_chapters(&mut self, id: String, cx: &mut Context<Self>) {
    let client = self.client.clone();
    let chapters = runtime::spawn(cx, async move { client.vod_chapters(&id).await });
    self._tasks.push(cx.spawn(async move |this, cx| {
      let chapters = match chapters.await {
        Some(Ok(chapters)) => play::chapters(&chapters),
        Some(Err(e)) => {
          tracing::warn!("VOD chapters: {e}");
          return;
        }
        None => return,
      };
      if chapters.is_empty() {
        return;
      }
      this
        .update(cx, |this, cx| {
          this
            .player
            .update(cx, |p, cx| p.set_chapters_with_pictures(chapters, cx));
        })
        .ok();
    }));
  }

  /// Saves the VOD's (or the live's recording's) position every [`progress::SAVE_EVERY`] while
  /// the page lives.
  fn save_periodically(&mut self, cx: &mut Context<Self>) {
    self._tasks.push(cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor().timer(progress::SAVE_EVERY).await;
        let saved = this.update(cx, |this, cx| {
          let player = this.player.clone();
          if let Some(saver) = &mut this.saver {
            saver.tick(&player, cx);
          }
          if let Some(saver) = &mut this.archive_saver
            && player.read(cx).rewound()
          {
            saver.tick(&player, cx);
          }
        });
        if saved.is_err() {
          break;
        }
      }
    }));
  }

  /// The clip's MP4s and its header.
  fn start_clip(&mut self, slug: String, cx: &mut Context<Self>) {
    let clicked = Instant::now();
    let (client, catalog) = (self.client.clone(), self.catalog.clone());
    let play = runtime::spawn(cx, {
      let slug = slug.clone();
      async move { client.clip(&slug).await }
    });
    let info = runtime::spawn(cx, async move {
      let clip = catalog.clip(&slug).await?;
      let user = catalog
        .users(std::slice::from_ref(&clip.broadcaster_id))
        .await?;
      Ok::<_, twitch::Error>((clip, user.into_iter().next()))
    });
    self._tasks.push(cx.spawn(async move |this, cx| {
      let (play, info) = (play.await, info.await);
      this
        .update(cx, |this, cx| {
          if let Some(Ok((clip, user))) = info {
            this.header = Some(Header {
              title: clip.title.clone(),
              name: clip.broadcaster_name.clone(),
              login: user.as_ref().map(|u| u.login.clone()).unwrap_or_default(),
              avatar: user.map(|u| u.profile_image_url),
              line: Some(tf!(
                "Clipped by {} · {} · {}",
                "Clip de {} · {} · {}",
                clip.creator_name,
                cards::views(clip.view_count),
                cards::date(clip.created_at)
              )),
              followed: None,
            });
            this.load_followed(cx);
            let title: SharedString = clip.title.into();
            this.nav.set_title(cx.entity_id(), title.clone(), cx);
            let author = this.header.as_ref().and_then(Header::author);
            this.player.update(cx, |p, cx| {
              p.set_title(title, cx);
              p.set_author(author, cx);
            });
          }
          match play {
            Some(Ok(clip)) => {
              let source = play::clip(&clip, Self::max_height(cx), &Settings::get(cx).player);
              this.start_source(source, clicked, cx);
            }
            Some(Err(e)) => this.fail(&e, cx),
            None => {}
          }
          cx.notify();
        })
        .ok();
    }));
  }

  /// The proxy's ad break, as the player's notice: only when the backup stream does not cover it.
  fn show_ad_notice(&mut self, cx: &mut Context<Self>) {
    let Some(state) = self.proxy.as_ref().map(Proxy::ad_state) else {
      return;
    };
    let notice: Option<SharedString> =
      (state.active && !state.covered).then(|| match state.duration {
        Some(duration) if duration > state.elapsed => {
          let left = (duration - state.elapsed).ceil();
          tf!(
            "Ad removed · back in {left:.0} s",
            "Publicité retirée · reprise dans {left:.0} s"
          )
          .into()
        }
        _ => t(
          "Ad removed · back in a moment",
          "Publicité retirée · reprise dans un instant",
        )
        .into(),
      });
    self
      .player
      .update(cx, |player, cx| player.set_notice(notice, cx));
  }

  fn show_channel(&mut self, channel: Channel, cx: &mut Context<Self>) {
    // Its raids and Hype Trains, once its id is known (a live only: VODs and clips have no
    // points view).
    if let Some(points) = self.points.clone() {
      let id = channel.id.clone();
      points.update(cx, |points, cx| points.watch_channel(id, cx));
    }
    let started = channel.live.as_ref().and_then(|l| l.started_at);
    let info = channel.live.as_ref().map(|live| LiveInfo {
      viewers: Some(live.viewers),
      started_at: started
        .and_then(|ms| u64::try_from(ms).ok())
        .map(|ms| UNIX_EPOCH + Duration::from_millis(ms)),
    });
    let title: SharedString = channel.title.clone().into();
    let author = Some(Author {
      name: channel.display_name.clone().into(),
      avatar: channel
        .avatar_url
        .clone()
        .map(|url| Picture::sized(move |_, _| url.clone())),
    });
    self.player.update(cx, |player, cx| {
      player.set_title(title, cx);
      player.set_author(author, cx);
      player.set_live_info(info, cx);
    });
    let line = channel.live.as_ref().map(|live| {
      let mut parts = Vec::new();
      if let Some(category) = &live.category {
        parts.push(category.clone());
      }
      parts.push(cards::viewers(live.viewers));
      parts.join(" · ")
    });
    self.started_at = started;
    self.header = Some(Header {
      title: channel.title,
      name: channel.display_name,
      login: channel.login,
      avatar: channel.avatar_url,
      line,
      followed: channel.followed,
    });
    cx.notify();
  }

  /// Reads whether the account follows the header's channel, for a VOD's or a clip's header
  /// (a live's comes with its channel). Nothing without an auth-token.
  fn load_followed(&mut self, cx: &mut Context<Self>) {
    let login = match &self.header {
      Some(header) if !header.login.is_empty() && self.client.signed_in() => header.login.clone(),
      _ => return,
    };
    let client = self.client.clone();
    let fetch = runtime::spawn(cx, async move { client.channel(&login).await });
    self
      ._tasks
      .push(cx.spawn(async move |this, cx| match fetch.await {
        Some(Ok(channel)) => {
          this
            .update(cx, |this, cx| {
              if let Some(header) = &mut this.header
                && header.login == channel.login
              {
                header.followed = channel.followed;
                cx.notify();
              }
            })
            .ok();
        }
        Some(Err(e)) => tracing::warn!("Twitch follow: {e}"),
        None => {}
      }));
  }

  fn header(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let accent = cx.theme().primary;
    let header = self.header.clone().unwrap_or_else(|| Header {
      name: self.login_hint(),
      ..Header::default()
    });
    let avatar = header
      .avatar
      .clone()
      .map(|url| picture::avatar(&Picture::sized(move |_, _| url.clone()), 48., window, cx));
    let uptime = self
      .started_at
      .and_then(|ms| u64::try_from(ms).ok())
      .and_then(|ms| {
        SystemTime::now()
          .duration_since(UNIX_EPOCH + Duration::from_millis(ms))
          .ok()
      })
      .map(|elapsed| {
        let time = ui::player::format_time(elapsed.as_secs_f64());
        tf!("live for {time}", "en direct depuis {time}")
      });
    let line = match (header.line.clone(), uptime) {
      (Some(line), Some(uptime)) => Some(format!("{line} · {uptime}")),
      (line, uptime) => line.or(uptime),
    };
    let (nav, login) = (self.nav.clone(), header.login.clone());
    h_flex()
      .gap_4()
      .items_start()
      .child(div().size(px(48.)).flex_none().children(avatar))
      .child(
        v_flex()
          .flex_1()
          .min_w_0()
          .gap_1()
          .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(
            if header.title.is_empty() {
              header.name.clone()
            } else {
              header.title.clone()
            },
          ))
          .child(
            div()
              .id("twitch-watch-channel")
              .text_sm()
              .font_weight(FontWeight::MEDIUM)
              .text_color(accent)
              .cursor_pointer()
              .hover(|el| el.underline())
              .on_aux_click(middle_click({
                let (nav, login) = (nav.clone(), login.clone());
                move |cx| {
                  if !login.is_empty() {
                    nav.open_in_new_tab(Route::Channel(login.clone()), None, cx);
                  }
                }
              }))
              .on_click(move |_, _, cx| {
                if !login.is_empty() {
                  nav.open(Route::Channel(login.clone()), cx);
                }
              })
              .child(header.name),
          )
          .children(line.map(|l| div().text_sm().text_color(theme::muted()).child(l))),
      )
      .when(!header.login.is_empty(), |el| {
        el.child(follow::follow_button(
          "watch",
          &header.login,
          header.followed,
        ))
      })
  }
}

impl Render for WatchView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    if !self.focused {
      self.focused = true;
      let focus = self.player.read(cx).focus_handle().clone();
      window.focus(&focus, cx);
    }
    if window.is_fullscreen() {
      return div()
        .size_full()
        .child(self.player.clone())
        .into_any_element();
    }
    let (theater, chat_hidden) = {
      let settings = Settings::get(cx);
      (settings.theater, settings.chat_hidden)
    };
    if let Some(chat) = &self.chat {
      chat.update(cx, |chat, cx| chat.set_collapsible(theater, cx));
    }
    let page = cx.weak_entity();
    ui::chat::chat_reopen_button(
      &self.player,
      theater && chat_hidden && self.chat.is_some(),
      move |_, cx| {
        Settings::update(cx, |s| s.chat_hidden = false);
        page.update(cx, |_, cx| cx.notify()).ok();
      },
      cx,
    );
    let header = self.header(window, cx).into_any_element();
    watch_layout(
      self.player.clone().into_any_element(),
      header,
      self.chat.clone().map(|chat| chat.into_any_element()),
      theater,
      chat_hidden,
    )
  }
}
