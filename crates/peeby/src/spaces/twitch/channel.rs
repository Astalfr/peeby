//! A channel's page: its header (Helix `users`), its live when on air (GQL `Channel`), then its
//! past broadcasts (`videos?type=archive`, with the account's resume positions as progress bars
//! and their game chapters, each playing the VOD from there)
//! or its most viewed clips (`clips`), in two tabs.

use std::collections::HashMap;

use gpui::prelude::*;
use gpui::{Context, FontWeight, Task, Window, div, px};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{h_flex, v_flex};
use twitch::gql::{self, Channel};
use twitch::helix::{Clip, Helix, User, Video};
use ui::i18n::t;
use ui::images::Images;
use ui::picture;
use ui::theme;

use super::browse::load;
use super::{Nav, Route, cards};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
  Videos,
  Clips,
}

struct Loaded {
  user: User,
  channel: Option<Channel>,
  videos: Vec<Video>,
  /// Where the viewer stopped in the VODs, seconds, by id.
  positions: HashMap<String, u32>,
  /// The VODs' game chapters, by id; the ones that kept one game are left out.
  chapters: HashMap<String, Vec<gql::Chapter>>,
  clips: Vec<Clip>,
}

pub struct ChannelView {
  nav: Nav,
  login: String,
  tab: Tab,
  loaded: Option<Result<Loaded, String>>,
  _load: Task<()>,
}

impl ChannelView {
  pub fn new(
    nav: Nav,
    helix: Helix,
    client: twitch::Client,
    login: String,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let task = load(
      cx,
      {
        let login = login.clone();
        async move {
          let user = helix.user(&login).await?;
          // The header's live and the lists are optional: their errors leave them out.
          let channel = client.channel(&login).await.ok();
          let videos = helix
            .videos(&user.id, Some("archive"), None)
            .await
            .map(|p| p.items)
            .unwrap_or_default();
          let ids: Vec<String> = videos.iter().map(|v| v.id.clone()).collect();
          let positions = client.vod_positions(&ids).await.unwrap_or_else(|e| {
            tracing::warn!("Twitch VOD positions: {e}");
            HashMap::new()
          });
          let chapters = client.vod_chapters_many(&ids).await.unwrap_or_else(|e| {
            tracing::warn!("Twitch VOD chapters: {e}");
            HashMap::new()
          });
          let clips = helix
            .clips(&user.id, None)
            .await
            .map(|p| p.items)
            .unwrap_or_default();
          Ok(Loaded {
            user,
            channel,
            videos,
            positions,
            chapters,
            clips,
          })
        }
      },
      |this: &mut Self, result, cx| {
        this.loaded = Some(result);
        cx.notify();
      },
      nav.clone(),
    );
    Self {
      nav,
      login,
      tab: Tab::Videos,
      loaded: None,
      _load: task,
    }
  }

  fn tab_button(&self, tab: Tab, label: &'static str, cx: &mut Context<Self>) -> Button {
    let id = match tab {
      Tab::Videos => "twitch-channel-tab-videos",
      Tab::Clips => "twitch-channel-tab-clips",
    };
    let button = Button::new(id)
      .label(label)
      .on_click(cx.listener(move |this, _, _, cx| {
        this.tab = tab;
        cx.notify();
      }));
    if self.tab == tab {
      button.primary()
    } else {
      button.ghost()
    }
  }
}

impl Render for ChannelView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let loaded = match &self.loaded {
      Some(Ok(loaded)) => loaded,
      Some(Err(error)) => {
        return super::browse::page(
          "twitch-channel",
          &self.login,
          None,
          cards::note(error.clone()).into_any_element(),
        )
        .into_any_element();
      }
      None => {
        return super::browse::page(
          "twitch-channel",
          &self.login,
          None,
          cards::note(t("Loading…", "Chargement…")).into_any_element(),
        )
        .into_any_element();
      }
    };
    let width = cards::stream_width(window, cx);
    let user = &loaded.user;
    let avatar = (!user.profile_image_url.is_empty())
      .then(|| picture::avatar(&cards::template(&user.profile_image_url), 80., window, cx));
    let live = loaded.channel.as_ref().and_then(|c| {
      let live = c.live.as_ref()?;
      let login = c.login.clone();
      let nav = self.nav.clone();
      let detail = [
        live.category.clone().unwrap_or_default(),
        cards::viewers(live.viewers),
      ]
      .into_iter()
      .filter(|s| !s.is_empty())
      .collect::<Vec<_>>()
      .join(" · ");
      Some(
        h_flex()
          .gap_4()
          .items_center()
          .p_4()
          .rounded(theme::RADIUS_LG)
          .bg(theme::surface())
          .border_1()
          .border_color(theme::border())
          .child(
            v_flex()
              .flex_1()
              .min_w_0()
              .gap_1()
              .child(
                div()
                  .text_xs()
                  .font_weight(FontWeight::BOLD)
                  .text_color(theme::accent_hover())
                  .child(t("LIVE", "EN DIRECT")),
              )
              .child(
                div()
                  .font_weight(FontWeight::MEDIUM)
                  .truncate()
                  .child(c.title.clone()),
              )
              .child(cards::note(detail)),
          )
          .child(
            Button::new("twitch-channel-watch")
              .primary()
              .label(t("Watch", "Regarder"))
              .on_click(move |_, _, cx| nav.open(Route::Watch(login.clone()), cx)),
          ),
      )
    });
    let list = match self.tab {
      Tab::Videos if loaded.videos.is_empty() => cards::note(t(
        "No public past broadcasts.",
        "Aucune rediffusion publique.",
      ))
      .into_any_element(),
      Tab::Videos => cards::wrap(
        loaded
          .videos
          .iter()
          .map(|v| {
            let position = loaded.positions.get(&v.id).copied();
            let chapters = loaded.chapters.get(&v.id).map(Vec::as_slice);
            cards::video_card(v, position, chapters, width, &self.nav, window, cx)
          })
          .collect(),
      )
      .into_any_element(),
      Tab::Clips if loaded.clips.is_empty() => {
        cards::note(t("No clips.", "Aucun clip.")).into_any_element()
      }
      Tab::Clips => cards::wrap(
        loaded
          .clips
          .iter()
          .map(|c| cards::clip_card(c, width, &self.nav, window, cx))
          .collect(),
      )
      .into_any_element(),
    };
    v_flex()
      .id("twitch-channel")
      .size_full()
      .overflow_y_scroll()
      .p_8()
      .gap_6()
      .child(
        h_flex()
          .gap_5()
          .items_center()
          .children(avatar.map(|a| div().size(px(80.)).flex_none().child(a)))
          .child(
            v_flex()
              .min_w_0()
              .gap_1()
              .child(
                div()
                  .text_2xl()
                  .font_weight(FontWeight::BOLD)
                  .child(user.display_name.clone()),
              )
              .when(!user.description.is_empty(), |el| {
                el.child(
                  div()
                    .text_sm()
                    .text_color(theme::muted())
                    .line_clamp(2)
                    .child(user.description.clone()),
                )
              }),
          ),
      )
      .children(live)
      .child(
        h_flex()
          .gap_2()
          .child(self.tab_button(Tab::Videos, t("Videos", "Vidéos"), cx))
          .child(self.tab_button(Tab::Clips, t("Clips", "Clips"), cx)),
      )
      .child(list)
      .into_any_element()
  }
}
