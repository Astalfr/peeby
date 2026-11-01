//! What plays in the Twitch space, with its header under the player:
//! - a channel's live: the shared player in live mode through the local proxy, the channel's
//!   header (GQL), a column for the chat beside it (step 9). While an ad break runs that the backup
//!   stream cannot cover, the player shows a notice with its countdown; the viewers are refreshed
//!   every minute;
//! - a VOD: through the proxy too, seekable, with the storyboard's previews on the seek bar;
//! - a clip: its direct MP4s.
//!
//! Theater mode puts the player across the page, as on YouTube (`Settings::theater`, shared). The
//! channel's name opens its page.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui::prelude::*;
use gpui::{
  Context, Entity, FontWeight, SharedString, Task, WeakEntity, Window, div, px, relative,
};
use gpui_component::{ActiveTheme, h_flex, v_flex};
use twitch::gql::Channel;
use twitch::helix::Helix;
use twitch::usher::Codec;
use twitch::{Proxy, Target};
use ui::images::Images;
use ui::picture::{self, Picture};
use ui::player::{LiveInfo, PlayerEvent, PlayerView, Source};
use ui::{runtime, theme};

use super::{Route, TwitchSpace, cards, play};
use crate::settings::{Player, Settings};

/// Width of the chat column.
const CHAT_WIDTH: f32 = 340.;
/// Theater mode: room left under the player, and its least height (as YouTube's watch page).
const THEATER_BELOW: f32 = 120.;
const THEATER_MIN: f32 = 240.;
/// How often a live's viewers and category are read again.
const INFO_EVERY: Duration = Duration::from_secs(60);

/// What the page plays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
  /// A channel's live, by login.
  Live(String),
  /// A VOD, by id.
  Vod(String),
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
}

pub struct WatchView {
  kind: Kind,
  client: twitch::Client,
  helix: Helix,
  nav: WeakEntity<TwitchSpace>,
  player: Entity<PlayerView>,
  /// Serves mpv's playlists (a live, a VOD); dropping it (with the page) stops it.
  proxy: Option<Proxy>,
  header: Option<Header>,
  /// A live's start, for the header's uptime.
  started_at: Option<i64>,
  focused: bool,
  _tasks: Vec<Task<()>>,
}

impl WatchView {
  pub fn new(
    kind: Kind,
    client: twitch::Client,
    helix: Helix,
    nav: WeakEntity<TwitchSpace>,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let settings = Settings::get(cx);
    let (prefs, theater) = (Player::prefs(settings), settings.theater);
    let player = cx.new(|cx| PlayerView::new(prefs, theater, cx));
    let live = matches!(kind, Kind::Live(_));
    cx.subscribe(
      &player,
      move |_, player, event: &PlayerEvent, cx| match *event {
        PlayerEvent::Theater => {
          Settings::update(cx, |s| s.theater = !s.theater);
          let theater = Settings::get(cx).theater;
          player.update(cx, |player, cx| player.set_theater(theater, cx));
          cx.notify();
        }
        PlayerEvent::Sound { volume, muted } => Player::save_sound(volume, muted, cx),
        PlayerEvent::Ended if live => player.update(cx, |player, cx| {
          player.set_notice(Some("Le live est terminé.".into()), cx)
        }),
        _ => {}
      },
    )
    .detach();
    let mut view = Self {
      kind,
      client,
      helix,
      nav,
      player,
      proxy: None,
      header: None,
      started_at: None,
      focused: false,
      _tasks: Vec::new(),
    };
    match view.kind.clone() {
      Kind::Live(login) => view.start_live(login, cx),
      Kind::Vod(id) => view.start_vod(id, cx),
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
      message
        .push_str(" Abonné ? Ajoutez le jeton du site Twitch dans les réglages pour la regarder.");
    }
    self.player.update(cx, |p, cx| p.fail(message.into(), cx));
  }

  fn login_hint(&self) -> String {
    match &self.kind {
      Kind::Live(login) => login.clone(),
      _ => "Cette chaîne".into(),
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
    let play = runtime::spawn(cx, {
      let (client, login) = (client.clone(), login.clone());
      async move { client.play(&Target::Live(login), &[Codec::H264]).await }
    });
    let channel = runtime::spawn(cx, async move { client.channel(&login).await });
    self._tasks.push(cx.spawn(async move |this, cx| {
      let (play, channel) = (play.await, channel.await);
      this
        .update(cx, |this, cx| {
          if let Some(Ok(channel)) = channel {
            this.show_channel(channel, cx);
          }
          match play {
            Some(Ok(proxy)) => {
              let source = play::proxied(
                &proxy,
                true,
                Self::max_height(cx),
                &Settings::get(cx).player,
              );
              this.proxy = Some(proxy);
              this.start_source(source, clicked, cx);
              this.watch_live(cx);
            }
            Some(Err(e)) => this.fail(&e, cx),
            None => {}
          }
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
        let Ok(fetch) = this.update(cx, |this, cx| {
          let client = this.client.clone();
          let login = this.login_hint();
          runtime::spawn(cx, async move { client.channel(&login).await })
        }) else {
          return;
        };
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

  /// The VOD through the proxy, its storyboard, and its header from Helix.
  fn start_vod(&mut self, id: String, cx: &mut Context<Self>) {
    let clicked = Instant::now();
    let (client, helix) = (self.client.clone(), self.helix.clone());
    let play = runtime::spawn(cx, {
      let (client, id) = (client.clone(), id.clone());
      async move { client.play(&Target::Vod(id), &[Codec::H264]).await }
    });
    let storyboard = runtime::spawn(cx, {
      let id = id.clone();
      async move { client.storyboard(&id).await }
    });
    let info = runtime::spawn(cx, async move {
      let video = helix.video(&id).await?;
      let avatar = cards::avatars(&helix, vec![video.user_id.clone()]).await;
      Ok::<_, twitch::Error>((video, avatar))
    });
    self._tasks.push(cx.spawn(async move |this, cx| {
      let (play, storyboard, info) = (play.await, storyboard.await, info.await);
      this
        .update(cx, |this, cx| {
          if let Some(Ok((video, avatars))) = info {
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
            });
            let title: SharedString = video.title.into();
            this.player.update(cx, |p, cx| p.set_title(title, cx));
          }
          match play {
            Some(Ok(proxy)) => {
              let mut source = play::proxied(
                &proxy,
                false,
                Self::max_height(cx),
                &Settings::get(cx).player,
              );
              source.previews = storyboard.and_then(Result::ok).and_then(play::previews);
              this.proxy = Some(proxy);
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

  /// The clip's MP4s and its header from Helix.
  fn start_clip(&mut self, slug: String, cx: &mut Context<Self>) {
    let clicked = Instant::now();
    let (client, helix) = (self.client.clone(), self.helix.clone());
    let play = runtime::spawn(cx, {
      let slug = slug.clone();
      async move { client.clip(&slug).await }
    });
    let info = runtime::spawn(cx, async move {
      let clip = helix.clip(&slug).await?;
      let user = helix
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
              line: Some(format!(
                "Clip de {} · {} · {}",
                clip.creator_name,
                cards::views(clip.view_count),
                cards::date(clip.created_at)
              )),
            });
            let title: SharedString = clip.title.into();
            this.player.update(cx, |p, cx| p.set_title(title, cx));
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
        Some(duration) if duration > state.elapsed => format!(
          "Publicité retirée · reprise dans {:.0} s",
          (duration - state.elapsed).ceil()
        )
        .into(),
        _ => "Publicité retirée · reprise dans un instant".into(),
      });
    self
      .player
      .update(cx, |player, cx| player.set_notice(notice, cx));
  }

  fn show_channel(&mut self, channel: Channel, cx: &mut Context<Self>) {
    let started = channel.live.as_ref().and_then(|l| l.started_at);
    let info = channel.live.as_ref().map(|live| LiveInfo {
      viewers: Some(live.viewers),
      started_at: started
        .and_then(|ms| u64::try_from(ms).ok())
        .map(|ms| UNIX_EPOCH + Duration::from_millis(ms)),
    });
    let title: SharedString = channel.display_name.clone().into();
    self.player.update(cx, |player, cx| {
      player.set_title(title, cx);
      player.set_live_info(info, cx);
    });
    let line = channel.live.as_ref().map(|live| {
      let mut parts = Vec::new();
      if let Some(category) = &live.category {
        parts.push(category.clone());
      }
      parts.push(format!("{} spectateurs", cards::grouped(live.viewers)));
      parts.join(" · ")
    });
    self.started_at = started;
    self.header = Some(Header {
      title: channel.title,
      name: channel.display_name,
      login: channel.login,
      avatar: channel.avatar_url,
      line,
    });
    cx.notify();
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
        format!(
          "en direct depuis {}",
          ui::player::format_time(elapsed.as_secs_f64())
        )
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
              .on_click(move |_, _, cx| {
                if !login.is_empty() {
                  nav
                    .update(cx, |space, cx| {
                      space.open(Route::Channel(login.clone()), cx)
                    })
                    .ok();
                }
              })
              .child(header.name),
          )
          .children(line.map(|l| div().text_sm().text_color(theme::muted()).child(l))),
      )
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
    let theater = Settings::get(cx).theater;
    let player = if theater {
      let viewport = window.viewport_size();
      let width = viewport.width - theme::sidebar_width(Settings::get(cx).sidebar_collapsed);
      let room = viewport.height - theme::TITLE_BAR_HEIGHT - px(THEATER_BELOW);
      div()
        .w_full()
        .flex_none()
        .h((width * (9. / 16.)).min(room.max(px(THEATER_MIN))))
        .rounded_tl(theme::PANEL_INNER_RADIUS)
        .bg(gpui::black())
        .child(self.player.clone())
    } else {
      // 16:9 through padding-top, as GPUI has no aspect-ratio style.
      div()
        .relative()
        .w_full()
        .pt(relative(9. / 16.))
        .rounded(theme::RADIUS_LG)
        .overflow_hidden()
        .child(div().absolute().inset_0().child(self.player.clone()))
    };
    let (above, inline) = if theater {
      (Some(player), None)
    } else {
      (None, Some(player))
    };
    // The chat column is a live's only (step 9).
    let chat = matches!(self.kind, Kind::Live(_)).then(|| {
      v_flex()
        .w(px(CHAT_WIDTH))
        .flex_none()
        .h(px(480.))
        .items_center()
        .justify_center()
        .rounded(theme::RADIUS_LG)
        .bg(theme::surface())
        .border_1()
        .border_color(theme::border())
        .child(
          div()
            .text_sm()
            .text_color(theme::muted())
            .child("Le chat arrive bientôt."),
        )
    });
    let header = self.header(window, cx);
    v_flex()
      .id("twitch-watch")
      .size_full()
      .overflow_y_scroll()
      .children(above)
      .child(
        h_flex()
          .w_full()
          .p_6()
          .gap_6()
          .items_start()
          .child(
            v_flex()
              .flex_1()
              .min_w_0()
              .gap_5()
              .children(inline)
              .child(header),
          )
          .children(chat),
      )
      .into_any_element()
  }
}
