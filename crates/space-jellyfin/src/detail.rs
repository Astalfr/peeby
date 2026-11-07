//! A movie's or a series' page: a full-width backdrop with the logo, details and synopsis; for a
//! series, its seasons as a pivot and the episodes of the one shown.

use std::time::Duration;

use gpui::prelude::*;
use gpui::{
  Animation, AnimationExt, Context, FontWeight, SharedString, Window, div, ease_out_quint,
  linear_color_stop, linear_gradient, px, relative,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::scroll::ScrollableElement;
use gpui_component::skeleton::Skeleton;
use gpui_component::{Disableable, Sizable, h_flex, v_flex};
use jellyfin_api::models::{BaseItem, ItemKind};
use jellyfin_api::{Client, Error, ItemId};
use ui::assets::Icon;
use ui::card::{WIDE, play_overlay, progress_bar};
use ui::i18n::t;
use ui::images::Images;
use ui::picture::{Picture, image_fill, logo};
use ui::text::{one_paragraph, plain_text};
use ui::{motion, runtime, tf, theme};

use super::items::{
  Shape, backdrop_ref, card_picture, clock, is_spoiler, logo_ref, progress, resumable, runtime,
  section_title,
};
use super::{Nav, Route, report};
use peeby_core::shell::space::middle_click;

const HERO_HEIGHT: f32 = 540.;
const EPISODE_THUMB_WIDTH: f32 = 224.;

pub struct DetailView {
  client: Client,
  nav: Nav,
  id: ItemId,
  item: Option<BaseItem>,
  seasons: Vec<BaseItem>,
  season: Option<ItemId>,
  episodes: Vec<BaseItem>,
  episodes_loading: bool,
}

impl DetailView {
  pub fn new(
    client: Client,
    nav: Nav,
    id: ItemId,
    season: Option<ItemId>,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let mut view = Self {
      client,
      nav,
      id,
      item: None,
      seasons: Vec::new(),
      season: None,
      episodes: Vec::new(),
      episodes_loading: false,
    };
    view.load(season, true, cx);
    view
  }

  /// Fetches the item again, e.g. after playback, keeping the shown season and the current
  /// content on screen until the new one arrives.
  pub fn reload(&mut self, cx: &mut Context<Self>) {
    self.load(self.season.clone(), false, cx);
  }

  /// Loads the item and its seasons, then the episodes of `season` (else of the first season
  /// with unwatched episodes, else the first). `skeleton` shows placeholders meanwhile.
  fn load(&mut self, season: Option<ItemId>, skeleton: bool, cx: &mut Context<Self>) {
    let id = self.id.clone();
    let fetch = runtime::spawn(cx, {
      let client = self.client.clone();
      async move {
        let item = client.item(&id).await?;
        let seasons = if item.kind == ItemKind::Series {
          client.seasons(&id).await?.items
        } else {
          Vec::new()
        };
        Ok::<_, Error>((item, seasons))
      }
    });
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |this, cx| {
          match result {
            Some(Ok((item, seasons))) => {
              let first = season
                .filter(|s| seasons.iter().any(|x| &x.id == s))
                .or_else(|| first_unwatched(&seasons))
                .or_else(|| seasons.first().map(|s| s.id.clone()));
              if let Some(name) = item.name.clone() {
                this.nav.set_title(cx.entity_id(), name.into(), cx);
              }
              this.item = Some(item);
              this.seasons = seasons;
              if let Some(first) = first {
                this.load_episodes(first, skeleton, cx);
              }
            }
            Some(Err(e)) => report(&this.nav, &e, cx),
            None => {}
          }
          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  fn load_episodes(&mut self, season: ItemId, skeleton: bool, cx: &mut Context<Self>) {
    let Some(series) = self.item.as_ref().map(|i| i.id.clone()) else {
      return;
    };
    self.season = Some(season.clone());
    self.episodes_loading = skeleton;
    let client = self.client.clone();
    let fetch = runtime::spawn(cx, async move { client.episodes(&series, &season).await });
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |this, cx| {
          this.episodes_loading = false;
          match result {
            Some(Ok(page)) => this.episodes = page.items,
            Some(Err(e)) => report(&this.nav, &e, cx),
            None => {}
          }
          cx.notify();
        })
        .ok();
    })
    .detach();
    cx.notify();
  }

  /// What the play button plays, and its label: the movie itself; for a series, the episode in
  /// progress, else the first unwatched one, else the first of the season shown.
  fn play_target(&self, item: &BaseItem) -> (Option<ItemId>, SharedString) {
    if item.kind != ItemKind::Series {
      let label = if resumable(item) {
        t("Resume", "Reprendre")
      } else {
        t("Play", "Lire")
      };
      return (Some(item.id.clone()), label.into());
    }
    let next = self
      .episodes
      .iter()
      .find(|e| resumable(e))
      .or_else(|| {
        self
          .episodes
          .iter()
          .find(|e| !e.user_data.as_ref().is_some_and(|u| u.played))
      })
      .or_else(|| self.episodes.first());
    match next {
      Some(episode) => {
        let verb = if resumable(episode) {
          t("Resume", "Reprendre")
        } else {
          t("Play", "Lire")
        };
        let number = episode
          .index_number
          .map(|n| format!(" E{n}"))
          .unwrap_or_default();
        (Some(episode.id.clone()), format!("{verb}{number}").into())
      }
      None => (None, t("Play", "Lire").into()),
    }
  }

  fn play(&self, id: ItemId, window: &mut Window, cx: &mut Context<Self>) {
    self.nav.open(Route::Play(id), window, cx);
  }

  fn render_hero(
    &mut self,
    item: &BaseItem,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement + use<> {
    let bg = theme::background();
    let width = f32::from(window.viewport_size().width);
    let backdrop =
      backdrop_ref(item).map_or(Picture::Sizes(Vec::new()), |b| b.picture(&self.client));
    let backdrop = image_fill(&backdrop, width, 16. / 9., px(0.), window, cx);
    let logo = logo_ref(item).and_then(|l| logo(&l.logo(&self.client), 480., 150., window, cx));
    let (play_target, play_label) = self.play_target(item);
    let genres = item.genres.clone().unwrap_or_default();

    let meta = h_flex()
      .gap_3()
      .items_center()
      .text_sm()
      .text_color(theme::foreground().opacity(0.85))
      .children(meta_parts(item).into_iter().map(|part| {
        match part {
          Meta::Text(text) => div().child(text).into_any_element(),
          Meta::Rating(rating) => div()
            .px_1p5()
            .rounded(px(4.))
            .border_1()
            .border_color(theme::foreground().opacity(0.4))
            .text_xs()
            .child(rating)
            .into_any_element(),
          Meta::Score(score) => h_flex()
            .gap_1()
            .items_center()
            .child(
              gpui_component::Icon::new(Icon::Star)
                .xsmall()
                .text_color(theme::accent()),
            )
            .child(format!("{score:.1}"))
            .into_any_element(),
        }
      }));

    div()
      .relative()
      .w_full()
      .h(px(HERO_HEIGHT))
      .flex_none()
      .overflow_hidden()
      .child(div().absolute().inset_0().child(backdrop))
      // Fade the backdrop into the page at the bottom and behind the text.
      .child(div().absolute().inset_0().bg(linear_gradient(
        90.,
        linear_color_stop(bg.opacity(0.95), 0.0),
        linear_color_stop(bg.opacity(0.0), 0.75),
      )))
      .child(div().absolute().inset_0().bg(linear_gradient(
        180.,
        linear_color_stop(bg.opacity(0.0), 0.35),
        linear_color_stop(bg, 1.0),
      )))
      .child(
        v_flex()
          .absolute()
          .left_8()
          .bottom(px(36.))
          .max_w(px(640.))
          .gap_3()
          .child(match logo {
            Some(logo) => logo,
            None => div()
              .text_size(px(40.))
              .font_weight(FontWeight::BOLD)
              .line_height(relative(1.1))
              .child(item.name.clone().unwrap_or_default())
              .into_any_element(),
          })
          .child(meta)
          .when(!genres.is_empty(), |el| {
            el.child(
              h_flex()
                .gap_2()
                .flex_wrap()
                .children(genres.into_iter().map(|genre| {
                  div()
                    .px_2p5()
                    .py_0p5()
                    .rounded_full()
                    .bg(gpui::white().opacity(0.08))
                    .text_xs()
                    .text_color(theme::foreground().opacity(0.85))
                    .child(genre)
                })),
            )
          })
          .child(
            h_flex().pt_2().child(
              Button::new("play")
                .primary()
                .large()
                .icon(Icon::Play)
                .label(play_label)
                .disabled(play_target.is_none())
                .on_click(cx.listener(move |this, _, window, cx| {
                  if let Some(id) = play_target.clone() {
                    this.play(id, window, cx);
                  }
                })),
            ),
          )
          // One paragraph, so the clamp holds (it counts the lines of each paragraph).
          .when_some(
            item.overview.as_deref().map(one_paragraph),
            |el, overview| {
              el.child(
                div()
                  .text_sm()
                  .line_clamp(4)
                  .text_ellipsis()
                  .text_color(theme::foreground().opacity(0.8))
                  .child(overview),
              )
            },
          )
          .with_animation(
            "detail-hero-text",
            Animation::new(Duration::from_millis(350)).with_easing(ease_out_quint()),
            |el, t| el.opacity(t).bottom(px(36. - 12. * (1. - t))),
          ),
      )
  }

  /// Seasons as a pivot: plain labels, the selected one underlined in the accent.
  fn render_seasons(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    h_flex()
      .gap_6()
      .flex_wrap()
      .border_b_1()
      .border_color(theme::border())
      .children(self.seasons.iter().map(|season| {
        let selected = self.season.as_ref() == Some(&season.id);
        let id = season.id.clone();
        v_flex()
          .id(SharedString::from(format!("season-{}", season.id)))
          .cursor_pointer()
          .gap_2()
          .pt_1()
          .text_color(if selected {
            theme::foreground()
          } else {
            theme::muted()
          })
          .when(selected, |el| el.font_weight(FontWeight::SEMIBOLD))
          .hover(|s| s.text_color(theme::foreground()))
          .on_click(cx.listener(move |this, _, _, cx| this.load_episodes(id.clone(), true, cx)))
          .child(season.name.clone().unwrap_or_default())
          .child(div().h(px(3.)).rounded_full().bg(if selected {
            theme::accent()
          } else {
            gpui::transparent_black()
          }))
      }))
  }

  fn render_episode(
    &self,
    episode: &BaseItem,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement + use<> {
    let spoiler = is_spoiler(episode, cx);
    let picture = card_picture(&self.client, episode, Shape::Thumb, cx);
    let thumb = image_fill(
      &picture,
      EPISODE_THUMB_WIDTH,
      WIDE,
      theme::RADIUS,
      window,
      cx,
    );
    let data = episode.user_data.as_ref();
    let played = data.is_some_and(|u| u.played);
    let resume_at = data
      .filter(|u| u.playback_position_ticks > 0 && !u.played)
      .map(|u| clock(u.playback_position_ticks));
    let title = match episode.index_number {
      Some(n) => format!("{n}. {}", episode.name.clone().unwrap_or_default()),
      None => episode.name.clone().unwrap_or_default(),
    };
    let duration = episode.run_time_ticks.map(runtime).unwrap_or_default();
    let group = SharedString::from(format!("episode-{}", episode.id));
    let hovered = motion::hover_state(group.clone(), window, cx);
    let hover = if *hovered.read(cx) { 1. } else { 0. };
    let id = episode.id.clone();
    let in_new_tab = {
      let (nav, route) = (self.nav.clone(), Route::Play(id.clone()));
      let label = episode.name.clone().map(SharedString::from);
      move |cx: &mut gpui::App| nav.open_in_new_tab(route.clone(), label.clone(), cx)
    };

    h_flex()
      .id(group.clone())
      .group(group.clone())
      .cursor_pointer()
      .on_hover(move |hover, window, cx| {
        hovered.update(cx, |h, _| *h = *hover);
        window.refresh();
      })
      .on_click(cx.listener(move |this, _, window, cx| this.play(id.clone(), window, cx)))
      .on_aux_click(middle_click(in_new_tab))
      .gap_5()
      .p_3()
      .rounded(theme::RADIUS)
      .items_start()
      .hover(|s| s.bg(theme::surface()))
      .child(
        div()
          .relative()
          .w(px(EPISODE_THUMB_WIDTH))
          .h(px(EPISODE_THUMB_WIDTH / WIDE))
          .flex_none()
          .rounded(theme::RADIUS)
          .child(thumb)
          // A play button over the thumbnail on hover.
          .child(play_overlay(hover, 44., theme::RADIUS))
          .children(progress(episode).map(progress_bar)),
      )
      .child(
        v_flex()
          .gap_1()
          .flex_1()
          .min_w_0()
          .child(
            h_flex()
              .gap_3()
              .items_center()
              .child(
                div()
                  .font_weight(FontWeight::SEMIBOLD)
                  .truncate()
                  .group_hover(group, |s| s.text_color(theme::accent_hover()))
                  .child(title),
              )
              .when(played, |el| {
                el.child(
                  h_flex()
                    .gap_1()
                    .items_center()
                    .text_xs()
                    .text_color(theme::accent())
                    .child(gpui_component::Icon::new(Icon::Check).xsmall())
                    .child(t("Watched", "Vu")),
                )
              })
              .child(div().flex_1())
              .child(
                div()
                  .text_xs()
                  .text_color(theme::muted())
                  .flex_none()
                  .child(duration),
              ),
          )
          .when_some(resume_at, |el, at| {
            el.child(
              div()
                .text_xs()
                .text_color(theme::accent_hover())
                .child(tf!("Resume at {at}", "Reprendre à {at}")),
            )
          })
          .when_some(
            episode
              .overview
              .as_deref()
              .filter(|_| !spoiler)
              .map(plain_text),
            |el, overview| {
              el.child(
                div()
                  .text_sm()
                  .text_color(theme::muted())
                  .line_clamp(2)
                  .child(overview),
              )
            },
          ),
      )
  }

  /// The banner and, for a series, its seasons and episodes.
  fn render_content(
    &mut self,
    item: &BaseItem,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let hero = self.render_hero(item, window, cx);
    let mut page = v_flex().child(hero);
    if item.kind == ItemKind::Series {
      let episodes: Vec<_> = self
        .episodes
        .clone()
        .iter()
        .map(|e| self.render_episode(e, window, cx).into_any_element())
        .collect();
      page = page.child(
        v_flex()
          .px_8()
          .pb_8()
          .gap_4()
          .child(section_title(t("Episodes", "Épisodes")))
          .when(self.seasons.len() > 1, |el| {
            el.child(self.render_seasons(cx))
          })
          .child(motion::crossfade(
            SharedString::from(format!("detail-episodes-{}", cx.entity_id())),
            self.episodes_loading,
            v_flex()
              .gap_1()
              .children((0..3).map(|_| skeleton_episode()))
              .into_any_element(),
            v_flex().gap_1().children(episodes).into_any_element(),
            window,
            cx,
          )),
      );
    }
    page.into_any_element()
  }
}

/// The first season that still has unwatched episodes.
fn first_unwatched(seasons: &[BaseItem]) -> Option<ItemId> {
  seasons
    .iter()
    .find(|s| {
      s.user_data
        .as_ref()
        .and_then(|u| u.unplayed_item_count)
        .is_some_and(|n| n > 0)
    })
    .map(|s| s.id.clone())
}

enum Meta {
  Text(String),
  Rating(String),
  Score(f32),
}

/// Year, duration (not for a series), parental rating, community rating, series status.
fn meta_parts(item: &BaseItem) -> Vec<Meta> {
  let mut parts = Vec::new();
  if let Some(year) = item.production_year {
    parts.push(Meta::Text(year.to_string()));
  }
  if let Some(ticks) = item
    .run_time_ticks
    .filter(|_| item.kind != ItemKind::Series)
  {
    parts.push(Meta::Text(runtime(ticks)));
  }
  if let Some(rating) = &item.official_rating {
    parts.push(Meta::Rating(rating.clone()));
  }
  if let Some(score) = item.community_rating {
    parts.push(Meta::Score(score));
  }
  if let Some(status) = item
    .status
    .as_deref()
    .filter(|_| item.kind == ItemKind::Series)
  {
    parts.push(Meta::Text(
      match status {
        "Continuing" => t("Continuing", "En cours"),
        "Ended" => t("Ended", "Terminée"),
        other => other,
      }
      .to_owned(),
    ));
  }
  parts
}

fn skeleton_episode() -> impl IntoElement {
  h_flex()
    .gap_5()
    .p_3()
    .child(
      Skeleton::new()
        .w(px(EPISODE_THUMB_WIDTH))
        .h(px(EPISODE_THUMB_WIDTH / WIDE))
        .rounded(theme::RADIUS),
    )
    .child(
      v_flex()
        .flex_1()
        .gap_2()
        .child(Skeleton::new().w(px(260.)).h(px(18.)))
        .child(Skeleton::new().w(px(420.)).h(px(14.))),
    )
}

impl Render for DetailView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let skeleton = v_flex()
      .child(Skeleton::new().w_full().h(px(HERO_HEIGHT)).rounded(px(0.)))
      .child(v_flex().px_8().children((0..3).map(|_| skeleton_episode())))
      .into_any_element();
    let content = match self.item.clone() {
      Some(item) => self.render_content(&item, window, cx),
      None => div().into_any_element(),
    };
    let body = motion::crossfade(
      SharedString::from(format!("detail-load-{}", cx.entity_id())),
      self.item.is_none(),
      skeleton,
      content,
      window,
      cx,
    );
    v_flex()
      .id("jellyfin-detail")
      .size_full()
      .overflow_y_scrollbar()
      .child(body)
  }
}
