//! Playback of a movie or an episode, in the content panel: negotiates Direct Play, gives the
//! shared player its [`Source`] (stream with the `Authorization` header, external subtitles and
//! their fonts, track names, trickplay previews), and reports playback to the server.
//!
//! What is Jellyfin's stays here (peeby's player page): reports (Playing on start, Progress every
//! 10 s and on pause, seek and track change, Stopped once, also when the page goes), the mapping
//! between Jellyfin's stream indexes and mpv's tracks with the choices carried from one episode
//! to the next, the server's media segments, and the previous and next episodes.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{AppContext, Context, Entity, SharedString, Subscription, Task, Window};
use jellyfin_api::media::{self, MediaSegment, TrickplayInfo, pick_trickplay};
use jellyfin_api::models::{BaseItem, ItemKind};
use jellyfin_api::playback::{MediaSource, MediaStream, PlaybackReport};
use jellyfin_api::{Client, Error, ItemId, Url};
use player::StartOptions;
use ui::player::segments::{Segment, SegmentKind};
use ui::player::{Layout, PlayerEvent, PlayerView, Previews, Source, Tile};
use ui::runtime;

use super::items::{TICKS_PER_SECOND, episode_number};
use super::login::describe;
use super::tracks::{StreamPref, SubtitlePref, TrackMap, TrackPrefs};
use super::{Nav, Route, report};
use crate::settings::{JellyfinPlayback, Player, Settings};
use crate::shell::space::Space;

const PROGRESS_INTERVAL: Duration = Duration::from_secs(10);
/// Trickplay thumbnails at least this wide are picked (the player draws them 176 px wide).
const TRICKPLAY_WIDTH: u32 = 200;

/// Everything the network side prepares before mpv starts.
struct Prepared {
  item: BaseItem,
  resume_ticks: i64,
  play_session_id: Option<String>,
  source: MediaSource,
  url: String,
  external_subtitles: Vec<String>,
  fonts_dir: Option<PathBuf>,
  map: TrackMap,
  trickplay: Option<TrickplayInfo>,
}

/// The file playing: what the reports and the track choices need.
struct Playing {
  source_id: String,
  play_session_id: Option<String>,
  map: TrackMap,
  series_id: Option<ItemId>,
  /// Selected streams, by Jellyfin index.
  audio: Option<i32>,
  subtitle: Option<i32>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReportKind {
  Playing,
  Progress,
}

pub struct PlayView {
  client: Client,
  nav: Nav,
  item_id: ItemId,
  player: Entity<PlayerView>,
  prefs: JellyfinPlayback,
  playing: Option<Playing>,
  previous: Option<BaseItem>,
  next: Option<BaseItem>,
  /// mpv loaded the file and the Playing report went; reset by the Stopped report.
  started: bool,
  /// The page entered fullscreen itself (setting), so leaving it hands that back.
  entered_fullscreen: bool,
  _tasks: Vec<Task<()>>,
  _player_events: Subscription,
}

impl PlayView {
  pub fn new(
    client: Client,
    nav: Nav,
    item_id: ItemId,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    let clicked = Instant::now();
    let settings = Settings::get(cx);
    let prefs = settings.jellyfin.playback.clone();
    let player_prefs = Player::prefs(settings);
    let player = cx.new(|cx| {
      let mut player = PlayerView::new(player_prefs, false, cx);
      player.set_layout(Layout::Page, cx);
      player
    });
    let focus = player.read(cx).focus_handle().clone();
    window.focus(&focus, cx);
    let entered_fullscreen = prefs.start_fullscreen && !window.is_fullscreen();
    if entered_fullscreen {
      window.toggle_fullscreen();
    }
    let player_events = cx.subscribe_in(&player, window, Self::on_player_event);
    // Whatever takes the page away (back, the sidebar, signing out, the next episode), the
    // server learns where playback stopped.
    cx.on_release(|this, cx| this.finish(false, cx)).detach();

    let prepare = runtime::spawn(cx, prepare(client.clone(), item_id.clone()));
    let segments = runtime::spawn(cx, {
      let client = client.clone();
      let item = item_id.clone();
      async move { client.media_segments(&item).await }
    });
    let start = cx.spawn(async move |this, cx| {
      let prepared = match prepare.await {
        Some(Ok(prepared)) => prepared,
        Some(Err(e)) => {
          this
            .update(cx, |this, cx| {
              report(&this.nav, &e, cx);
              let message = describe(&e);
              this.player.update(cx, |p, cx| p.fail(message, cx));
            })
            .ok();
          return;
        }
        None => return,
      };
      let Ok(adjacent) = this.update(cx, |this, cx| {
        let adjacent = this.load_adjacent(&prepared.item, cx);
        this.start(prepared, clicked, cx);
        adjacent
      }) else {
        return;
      };
      // Extras, best effort: playback has started without them.
      match segments.await {
        Some(Ok(list)) => {
          this
            .update(cx, |this, cx| {
              let segments = from_server(&list);
              this.player.update(cx, |p, cx| p.set_segments(segments, cx));
            })
            .ok();
        }
        Some(Err(e)) => tracing::warn!("media segments: {e}"),
        None => {}
      }
      if let Some(adjacent) = adjacent
        && let Some(Ok(adjacent)) = adjacent.await
      {
        this
          .update(cx, |this, cx| this.set_adjacent(adjacent, cx))
          .ok();
      }
    });
    let progress = cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor().timer(PROGRESS_INTERVAL).await;
        if this
          .update(cx, |this, cx| this.send_report(ReportKind::Progress, cx))
          .is_err()
        {
          break;
        }
      }
    });

    Self {
      client,
      nav,
      item_id,
      player,
      prefs,
      playing: None,
      previous: None,
      next: None,
      started: false,
      entered_fullscreen,
      _tasks: vec![start, progress],
      _player_events: player_events,
    }
  }

  /// The player, for the shell's mini-player.
  pub fn player(&self) -> Entity<PlayerView> {
    self.player.clone()
  }

  /// The previous and next episodes of an episode, fetched in the background.
  fn load_adjacent(
    &self,
    item: &BaseItem,
    cx: &mut Context<Self>,
  ) -> Option<impl Future<Output = Option<Result<media::Adjacent, Error>>> + use<>> {
    let series = item
      .series_id
      .clone()
      .filter(|_| item.kind == ItemKind::Episode)?;
    let (client, episode) = (self.client.clone(), item.id.clone());
    Some(runtime::spawn(cx, async move {
      client.adjacent_episodes(&series, &episode).await
    }))
  }

  fn set_adjacent(&mut self, adjacent: media::Adjacent, cx: &mut Context<Self>) {
    self.previous = adjacent.previous;
    self.next = adjacent.next;
    let queue = (self.previous.is_some(), self.next.is_some());
    let up_next = self.next.as_ref().map(|next| {
      SharedString::from(format!(
        "{}{}",
        episode_number(next),
        next.name.clone().unwrap_or_default()
      ))
    });
    self.player.update(cx, |p, cx| {
      p.set_queue(Some(queue), cx);
      p.set_up_next(up_next, cx);
    });
  }

  /// Picks the tracks (the series' last choice, else the languages of the settings, else the
  /// server's) and starts mpv.
  fn start(&mut self, prepared: Prepared, clicked: Instant, cx: &mut Context<Self>) {
    let Prepared {
      item,
      resume_ticks,
      play_session_id,
      source,
      url,
      external_subtitles,
      fonts_dir,
      map,
      trickplay,
    } = prepared;
    let series_id = item.series_id.clone();
    let track_prefs = series_id
      .as_ref()
      .and_then(|series| self.nav.upgrade()?.read(cx).track_prefs(series).cloned())
      .unwrap_or_else(|| language_prefs(&self.prefs));
    let (audio, subtitle) = map.apply(
      &track_prefs,
      source.default_audio_stream_index,
      source.default_subtitle_stream_index,
    );
    let mut options = StartOptions {
      url,
      http_headers: vec![format!(
        "Authorization: {}",
        self.client.authorization_header()
      )],
      start_seconds: resume_ticks as f64 / TICKS_PER_SECOND as f64,
      audio: Some(map.mpv_audio(audio)),
      subtitle: Some(map.mpv_subtitle(subtitle)),
      external_subtitles,
      fonts_dir,
      subtitle_scale: (self.prefs.subtitle_scale != 1.0).then_some(self.prefs.subtitle_scale),
      buffer_mb: Some(self.prefs.network_buffer_mb),
      ..Default::default()
    };
    Settings::get(cx).player.apply(&mut options);
    let previews = trickplay.map(|info| {
      Rc::new(Trickplay {
        client: self.client.clone(),
        item: item.id.clone(),
        source_id: source.id.clone(),
        info,
      }) as Rc<dyn Previews>
    });
    let player_source = Source {
      options,
      previews,
      audio_names: map.audio().iter().map(|t| t.label.clone()).collect(),
      subtitle_names: map.subtitles().iter().map(|t| t.label.clone()).collect(),
      ..Default::default()
    };
    let (title, subtitle_line) = titles(&item);
    self.nav.set_title(cx.entity_id(), title.clone().into(), cx);
    self.playing = Some(Playing {
      source_id: source.id,
      play_session_id,
      map,
      series_id,
      audio,
      subtitle,
    });
    self.player.update(cx, |p, cx| {
      p.set_title(title.into(), cx);
      p.set_subtitle(subtitle_line.into(), cx);
      p.start(player_source, clicked, cx);
    });
  }

  fn on_player_event(
    &mut self,
    _: &Entity<PlayerView>,
    event: &PlayerEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    match event {
      PlayerEvent::Started => {
        self.started = true;
        self.send_report(ReportKind::Playing, cx);
      }
      PlayerEvent::Paused(_) | PlayerEvent::Seeked => self.send_report(ReportKind::Progress, cx),
      PlayerEvent::TracksChanged { audio, subtitle } => {
        if let Some(playing) = &mut self.playing {
          let index = |tracks: &[super::tracks::TrackEntry], id: Option<i64>| {
            id.and_then(|id| tracks.get(usize::try_from(id - 1).ok()?))
              .map(|t| t.index)
          };
          playing.audio = index(playing.map.audio(), *audio);
          playing.subtitle = index(playing.map.subtitles(), *subtitle);
        }
        self.send_report(ReportKind::Progress, cx);
      }
      PlayerEvent::Sound { volume, muted } => Player::save_sound(*volume, *muted, cx),
      PlayerEvent::Ended => {
        self.finish(true, cx);
        self.close(cx);
      }
      PlayerEvent::Next => {
        // Only an episode left in its credits counts as watched; N midway does not.
        let watched = self.player.read(cx).finished();
        if let Some(next) = self.next.clone() {
          self.play_episode(next, watched, window, cx);
        }
      }
      PlayerEvent::Previous => {
        if let Some(previous) = self.previous.clone() {
          self.play_episode(previous, false, window, cx);
        }
      }
      PlayerEvent::Close => self.close(cx),
      // The page has no theater mode: the player is the whole page. Jellyfin has no lives.
      PlayerEvent::Theater | PlayerEvent::Rewound { .. } => {}
    }
  }

  /// Back to the page underneath.
  fn close(&mut self, cx: &mut Context<Self>) {
    self.remember_tracks(cx);
    let (leave_fullscreen, tab) = (self.entered_fullscreen, self.nav.tab);
    self
      .nav
      .update(cx, |space, cx| {
        if leave_fullscreen {
          space.leave_fullscreen();
        }
        space.back(tab, cx)
      })
      .ok();
  }

  /// Replaces this page by a player for another episode of the series.
  fn play_episode(
    &mut self,
    episode: BaseItem,
    watched: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.remember_tracks(cx);
    self.finish(watched, cx);
    // The next player finds the window fullscreen already: hand over leaving it.
    let (leave_fullscreen, tab) = (self.entered_fullscreen, self.nav.tab);
    self
      .nav
      .update(cx, |space, cx| {
        if leave_fullscreen {
          space.leave_fullscreen();
        }
        space.replace(tab, Route::Play(episode.id), window, cx)
      })
      .ok();
  }

  /// Keeps the series' track choices for its next episodes.
  fn remember_tracks(&self, cx: &mut gpui::App) {
    let Some(playing) = &self.playing else {
      return;
    };
    let Some(series) = playing.series_id.clone() else {
      return;
    };
    let prefs = playing.map.prefs(playing.audio, playing.subtitle);
    self
      .nav
      .update(cx, |space, _| space.remember_tracks(series, prefs))
      .ok();
  }

  fn report(&self, cx: &gpui::App) -> Option<PlaybackReport> {
    let playing = self.playing.as_ref()?;
    let player = self.player.read(cx);
    Some(PlaybackReport {
      item_id: self.item_id.clone(),
      media_source_id: playing.source_id.clone(),
      play_session_id: playing.play_session_id.clone(),
      position_ticks: (player.position() * TICKS_PER_SECOND as f64) as i64,
      is_paused: player.is_paused(),
      audio_stream_index: playing.audio,
      subtitle_stream_index: playing.subtitle,
      play_method: "DirectPlay",
      can_seek: true,
    })
  }

  fn send_report(&mut self, kind: ReportKind, cx: &mut gpui::App) {
    if !self.started {
      return;
    }
    let Some(report) = self.report(cx) else {
      return;
    };
    let client = self.client.clone();
    drop(runtime::spawn(cx, async move {
      let result = match kind {
        ReportKind::Playing => client.report_playing(&report).await,
        ReportKind::Progress => client.report_progress(&report).await,
      };
      if let Err(e) = result {
        tracing::warn!("Jellyfin playback report: {e}");
      }
    }));
  }

  /// Sends the Stopped report once; `watched` also marks the item played (the server does it
  /// past about 90 % only, and credits can start earlier). Then the page underneath reloads.
  fn finish(&mut self, watched: bool, cx: &mut gpui::App) {
    if !self.started {
      return;
    }
    self.started = false;
    let Some(report) = self.report(cx) else {
      return;
    };
    let client = self.client.clone();
    let item = self.item_id.clone();
    let sent = runtime::spawn(cx, async move {
      if let Err(e) = client.report_stopped(&report).await {
        tracing::warn!("Jellyfin playback report: {e}");
      }
      if watched && let Err(e) = client.mark_played(&item).await {
        tracing::warn!("cannot mark the item watched: {e}");
      }
    });
    // The page underneath shows the new watched state and progress, which the server only has
    // once the report went through.
    let nav = self.nav.clone();
    cx.spawn(async move |cx| {
      sent.await;
      let tab = nav.tab;
      nav.update(cx, |space, cx| space.refresh(tab, cx)).ok();
    })
    .detach();
  }
}

impl Render for PlayView {
  fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
    gpui::div().size_full().child(self.player.clone())
  }
}

/// The player's title and the line under it: the series and "S1:E2 · name" for an episode,
/// else the name and the year.
fn titles(item: &BaseItem) -> (String, String) {
  let name = item.name.clone().unwrap_or_default();
  match item.kind {
    ItemKind::Episode => (
      item.series_name.clone().unwrap_or_default(),
      format!("{}{name}", episode_number(item)),
    ),
    _ => (
      name,
      item
        .production_year
        .map(|y| y.to_string())
        .unwrap_or_default(),
    ),
  }
}

/// Network side: the item, Direct Play negotiation (anything else is refused with the server's
/// reasons), the stream and external subtitle URLs, and the fonts of external ASS subtitles.
async fn prepare(client: Client, item_id: ItemId) -> Result<Prepared, Error> {
  let item = client.item(&item_id).await?;
  let resume_ticks = item
    .user_data
    .as_ref()
    .filter(|u| !u.played)
    .map(|u| u.playback_position_ticks)
    .unwrap_or(0);
  let info = client.playback_info(&item_id, resume_ticks).await?;
  let play_session_id = info.play_session_id.clone();
  let source = info
    .media_sources
    .into_iter()
    .next()
    .ok_or_else(|| Error::PlaybackRefused {
      code: "NoMediaSource".into(),
    })?;
  source.ensure_direct_play()?;
  let url = client
    .stream_url(&item_id, &source, play_session_id.as_deref())?
    .to_string();
  let map = TrackMap::new(&source);
  let external_subtitles = map
    .external_subtitles()
    .iter()
    .map(|stream| subtitle_url(&client, &item_id, &source, stream).map(|u| u.to_string()))
    .collect::<Result<Vec<_>, _>>()?;
  let needs_fonts = map
    .external_subtitles()
    .iter()
    .any(|s| matches!(s.codec.as_deref(), Some("ass" | "ssa")));
  let fonts_dir = if needs_fonts {
    download_fonts(&client, &item_id, &source).await
  } else {
    None
  };
  let trickplay = item
    .trickplay
    .as_ref()
    .and_then(|manifest| pick_trickplay(manifest, &source.id, TRICKPLAY_WIDTH))
    .cloned();
  Ok(Prepared {
    item,
    resume_ticks,
    play_session_id,
    source,
    url,
    external_subtitles,
    fonts_dir,
    map,
    trickplay,
  })
}

/// An external subtitle's URL: its delivery URL, else the subtitle stream endpoint.
fn subtitle_url(
  client: &Client,
  item: &ItemId,
  source: &MediaSource,
  stream: &MediaStream,
) -> Result<Url, Error> {
  match &stream.delivery_url {
    Some(path) => client.delivery_url(path),
    None => {
      let ext = stream.codec.as_deref().unwrap_or("srt");
      client.delivery_url(&format!(
        "Videos/{item}/{}/Subtitles/{}/0/Stream.{ext}",
        source.id, stream.index
      ))
    }
  }
}

/// Downloads the font attachments into the cache (`%LOCALAPPDATA%\peeby\fonts\<item>`), for
/// external .ass files that use them. Best effort.
async fn download_fonts(client: &Client, item: &ItemId, source: &MediaSource) -> Option<PathBuf> {
  let dir = dirs::cache_dir()?
    .join("peeby")
    .join("fonts")
    .join(item.as_str());
  std::fs::create_dir_all(&dir).ok()?;
  let mut count = 0;
  for attachment in source.media_attachments.iter().filter(|a| a.is_font()) {
    let name = attachment
      .file_name
      .clone()
      .unwrap_or_else(|| format!("font-{}.ttf", attachment.index))
      .replace(['/', '\\', ':'], "_");
    let path = dir.join(name);
    if path.exists() {
      count += 1;
      continue;
    }
    let Ok(url) = client.attachment_url(item, source, attachment) else {
      continue;
    };
    match client.fetch_bytes(&url).await {
      Ok(bytes) => match std::fs::write(&path, bytes) {
        Ok(()) => count += 1,
        Err(e) => tracing::warn!("cannot save the font {}: {e}", path.display()),
      },
      Err(e) => tracing::warn!("font attachment {}: {e}", attachment.index),
    }
  }
  (count > 0).then_some(dir)
}

/// The server's media segments as the player's; unknown kinds are left out.
fn from_server(segments: &[MediaSegment]) -> Vec<Segment> {
  segments
    .iter()
    .filter_map(|s| {
      let kind = match s.kind {
        media::SegmentKind::Intro => SegmentKind::Intro,
        media::SegmentKind::Recap => SegmentKind::Recap,
        media::SegmentKind::Outro => SegmentKind::Outro,
        media::SegmentKind::Preview => SegmentKind::Preview,
        media::SegmentKind::Commercial => SegmentKind::Commercial,
        media::SegmentKind::Unknown => return None,
      };
      Some(Segment {
        kind,
        start: s.start_seconds(),
        end: s.end_seconds(),
      })
    })
    .collect()
}

/// Track choices from the languages of the settings.
fn language_prefs(prefs: &JellyfinPlayback) -> TrackPrefs {
  let like = |language: &str| StreamPref {
    language: Some(language.to_owned()),
    title: None,
    forced: false,
  };
  TrackPrefs {
    audio: prefs.audio_language.as_deref().map(like),
    subtitle: match prefs.subtitle_language.as_deref() {
      None => SubtitlePref::Default,
      Some("off") => SubtitlePref::Off,
      Some(language) => SubtitlePref::Like(like(language)),
    },
  }
}

/// Jellyfin's trickplay thumbnails as the seek bar's previews.
struct Trickplay {
  client: Client,
  item: ItemId,
  source_id: String,
  info: TrickplayInfo,
}

impl Previews for Trickplay {
  fn tile(&self, seconds: f64, _duration: f64) -> Option<Tile> {
    let cell = self.info.cell_at(seconds)?;
    let url = self
      .client
      .trickplay_tile_url(&self.item, &self.source_id, self.info.width, cell.tile)
      .ok()?;
    Some(Tile {
      url: url.to_string(),
      column: cell.x / self.info.width.max(1),
      row: cell.y / self.info.height.max(1),
    })
  }

  fn tile_size(&self) -> (u32, u32) {
    (self.info.width, self.info.height)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn server_segments_keep_known_kinds() {
    let list: Vec<MediaSegment> = serde_json::from_value(serde_json::json!([
      { "Type": "Intro", "StartTicks": 600_000_000i64, "EndTicks": 1_500_000_000i64 },
      { "Type": "Mystery", "StartTicks": 0, "EndTicks": 10 },
    ]))
    .unwrap();
    assert_eq!(
      from_server(&list),
      vec![Segment {
        kind: SegmentKind::Intro,
        start: 60.0,
        end: 150.0
      }]
    );
  }

  #[test]
  fn languages_become_track_prefs() {
    let prefs = JellyfinPlayback {
      audio_language: Some("jpn".into()),
      subtitle_language: Some("off".into()),
      ..Default::default()
    };
    let track = language_prefs(&prefs);
    assert_eq!(track.audio.unwrap().language.as_deref(), Some("jpn"));
    assert_eq!(track.subtitle, SubtitlePref::Off);
  }
}
