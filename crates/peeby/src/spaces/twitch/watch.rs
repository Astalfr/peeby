//! A channel's live: the shared player in live mode, fed by the local proxy, with the channel's
//! header under it and a column for the chat beside it (step 9). Theater mode puts the player
//! across the page, as on YouTube (`Settings::theater`, shared).
//!
//! While an ad break runs that the backup stream cannot cover, the player shows a notice with its
//! countdown; the viewers are refreshed every minute.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui::prelude::*;
use gpui::{
  Context, Entity, FontWeight, SharedString, Task, WeakEntity, Window, div, px, relative,
};
use gpui_component::{ActiveTheme, h_flex, v_flex};
use twitch::gql::Channel;
use twitch::usher::Codec;
use twitch::{Proxy, Target};
use ui::images::Images;
use ui::picture::{self, Picture};
use ui::player::{LiveInfo, PlayerEvent, PlayerView};
use ui::{runtime, theme};

use super::{TwitchSpace, play};
use crate::settings::{Player, Settings};

/// Width of the chat column.
const CHAT_WIDTH: f32 = 340.;
/// Theater mode: room left under the player, and its least height (as YouTube's watch page).
const THEATER_BELOW: f32 = 120.;
const THEATER_MIN: f32 = 240.;
/// How often the viewers and the category are read again.
const INFO_EVERY: Duration = Duration::from_secs(60);

pub struct WatchView {
  login: String,
  client: twitch::Client,
  player: Entity<PlayerView>,
  /// Serves mpv's playlists; dropping it (with the page) stops it.
  proxy: Option<Proxy>,
  channel: Option<Channel>,
  focused: bool,
  _tasks: Vec<Task<()>>,
}

impl WatchView {
  pub fn new(
    login: String,
    client: twitch::Client,
    _nav: WeakEntity<TwitchSpace>,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let settings = Settings::get(cx);
    let (prefs, theater) = (Player::prefs(settings), settings.theater);
    let player = cx.new(|cx| PlayerView::new(prefs, theater, cx));
    cx.subscribe(&player, |_, player, event: &PlayerEvent, cx| match *event {
      PlayerEvent::Theater => {
        Settings::update(cx, |s| s.theater = !s.theater);
        let theater = Settings::get(cx).theater;
        player.update(cx, |player, cx| player.set_theater(theater, cx));
        cx.notify();
      }
      PlayerEvent::Sound { volume, muted } => Player::save_sound(volume, muted, cx),
      PlayerEvent::Ended => player.update(cx, |player, cx| {
        player.set_notice(Some("Le live est terminé.".into()), cx)
      }),
      _ => {}
    })
    .detach();
    let mut view = Self {
      login,
      client,
      player,
      proxy: None,
      channel: None,
      focused: false,
      _tasks: Vec::new(),
    };
    view.start(cx);
    view
  }

  /// The player, for the shell's mini-player.
  pub fn player(&self) -> Entity<PlayerView> {
    self.player.clone()
  }

  /// Asks for the playback (tokens, masters, proxy) and the channel's header together.
  fn start(&mut self, cx: &mut Context<Self>) {
    let clicked = Instant::now();
    let (client, login) = (self.client.clone(), self.login.clone());
    let play = runtime::spawn(cx, {
      let (client, login) = (client.clone(), login.clone());
      async move { client.play(&Target::Live(login), &[Codec::H264]).await }
    });
    let channel = runtime::spawn(cx, async move { client.channel(&login).await });
    let task = cx.spawn(async move |this, cx| {
      let (play, channel) = (play.await, channel.await);
      this
        .update(cx, |this, cx| {
          if let Some(Ok(channel)) = channel {
            this.show_channel(channel, cx);
          }
          match play {
            Some(Ok(proxy)) => this.play(proxy, clicked, cx),
            Some(Err(e)) => {
              tracing::warn!("Twitch playback of {}: {e}", this.login);
              let name = this.shown_name();
              this
                .player
                .update(cx, |p, cx| p.fail(play::describe(&e, &name).into(), cx));
            }
            None => {}
          }
        })
        .ok();
    });
    self._tasks.push(task);
  }

  fn play(&mut self, proxy: Proxy, clicked: Instant, cx: &mut Context<Self>) {
    let settings = Settings::get(cx);
    let max_height = settings.twitch.max_height.unwrap_or(play::DEFAULT_HEIGHT);
    let source = play::source(&proxy, max_height, &settings.player);
    self.proxy = Some(proxy);
    self
      .player
      .update(cx, |player, cx| player.start(source, clicked, cx));
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
          let (client, login) = (this.client.clone(), this.login.clone());
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
    let info = channel.live.as_ref().map(|live| LiveInfo {
      viewers: Some(live.viewers),
      started_at: live
        .started_at
        .and_then(|ms| u64::try_from(ms).ok())
        .map(|ms| UNIX_EPOCH + Duration::from_millis(ms)),
    });
    let title: SharedString = channel.display_name.clone().into();
    self.player.update(cx, |player, cx| {
      player.set_title(title, cx);
      player.set_live_info(info, cx);
    });
    self.channel = Some(channel);
    cx.notify();
  }

  /// The channel's shown name, else its login.
  fn shown_name(&self) -> String {
    self
      .channel
      .as_ref()
      .map_or_else(|| self.login.clone(), |c| c.display_name.clone())
  }

  fn header(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let accent = cx.theme().primary;
    let channel = self.channel.clone();
    let name = self.shown_name();
    let avatar = channel
      .as_ref()
      .and_then(|c| c.avatar_url.clone())
      .map(|url| picture::avatar(&Picture::sized(move |_, _| url.clone()), 48., window, cx));
    let title = channel
      .as_ref()
      .map(|c| c.title.clone())
      .unwrap_or_default();
    let line = channel.as_ref().and_then(|c| c.live.as_ref()).map(|live| {
      let mut parts = Vec::new();
      if let Some(category) = &live.category {
        parts.push(category.clone());
      }
      parts.push(viewers(live.viewers));
      if let Some(start) = live.started_at
        && let Some(elapsed) = u64::try_from(start).ok().and_then(|ms| {
          SystemTime::now()
            .duration_since(UNIX_EPOCH + Duration::from_millis(ms))
            .ok()
        })
      {
        parts.push(format!(
          "en direct depuis {}",
          ui::player::format_time(elapsed.as_secs_f64())
        ));
      }
      parts.join(" · ")
    });
    h_flex()
      .gap_4()
      .items_start()
      .child(div().size(px(48.)).flex_none().children(avatar))
      .child(
        v_flex()
          .flex_1()
          .min_w_0()
          .gap_1()
          .child(
            div()
              .text_lg()
              .font_weight(FontWeight::SEMIBOLD)
              .child(if title.is_empty() {
                name.clone()
              } else {
                title
              }),
          )
          .child(
            div()
              .text_sm()
              .font_weight(FontWeight::MEDIUM)
              .text_color(accent)
              .child(name),
          )
          .children(line.map(|l| div().text_sm().text_color(theme::muted()).child(l))),
      )
  }
}

/// `80 308 spectateurs`.
fn viewers(n: u64) -> String {
  let digits = n.to_string();
  let mut grouped = String::new();
  for (i, d) in digits.chars().enumerate() {
    if i > 0 && (digits.len() - i).is_multiple_of(3) {
      grouped.push('\u{202f}');
    }
    grouped.push(d);
  }
  if n > 1 {
    format!("{grouped} spectateurs")
  } else {
    format!("{grouped} spectateur")
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
    let chat = v_flex()
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
      );
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
          .child(chat),
      )
      .into_any_element()
  }
}
