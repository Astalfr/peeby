//! Home: a banner (what to resume or watch next), then "Reprendre", "À suivre" and the latest
//! additions of each library.

use std::time::Duration;

use gpui::prelude::*;
use gpui::{
  Animation, AnimationExt, AnyElement, Context, FontWeight, SharedString, Task, WeakEntity, Window,
  div, ease_out_quint, linear_color_stop, linear_gradient, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::scroll::ScrollableElement;
use gpui_component::skeleton::Skeleton;
use gpui_component::{Sizable, h_flex, v_flex};
use jellyfin_api::models::{BaseItem, CollectionType, ItemKind};
use jellyfin_api::{Client, Error, NextUpOptions};
use ui::assets::Icon;
use ui::card::skeleton_poster;
use ui::i18n::t;
use ui::images::Images;
use ui::picture::{Picture, image_fill, logo};
use ui::rows::scroll_row;
use ui::text::one_paragraph;
use ui::{motion, runtime, tf, theme};

use super::items::{
  Shape, TICKS_PER_SECOND, backdrop_ref, card, episode_number, is_spoiler, logo_ref, resumable,
  route_for, section_title,
};
use super::{JellyfinSpace, Route, report};
use crate::settings::Settings;

const ROW_LIMIT: u32 = 16;
/// Titles in the banner.
const HERO_COUNT: usize = 5;
const HERO_INTERVAL: Duration = Duration::from_secs(8);
const HERO_HEIGHT: f32 = 440.;
/// Cross-fade between two banner backdrops.
const HERO_FADE: Duration = Duration::from_millis(700);

struct Row {
  title: SharedString,
  shape: Shape,
  items: Vec<BaseItem>,
  /// A click starts playback instead of opening the title's page.
  play: bool,
}

pub struct HomeView {
  client: Client,
  nav: WeakEntity<JellyfinSpace>,
  rows: Vec<Row>,
  loading: bool,
  hero: Vec<BaseItem>,
  hero_index: usize,
  /// The slide being faded out, under the current one.
  hero_previous: Option<usize>,
  hero_hovered: bool,
  _rotation: Option<Task<()>>,
}

impl HomeView {
  pub fn new(client: Client, nav: WeakEntity<JellyfinSpace>, cx: &mut Context<Self>) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let mut view = Self {
      client,
      nav,
      rows: Vec::new(),
      loading: true,
      hero: Vec::new(),
      hero_index: 0,
      hero_previous: None,
      hero_hovered: false,
      _rotation: None,
    };
    view.load(cx);
    view
  }

  /// Fetches the rows again, e.g. after playback; the current ones stay on screen meanwhile.
  pub fn reload(&mut self, cx: &mut Context<Self>) {
    self.load(cx);
  }

  /// Loads "Reprendre", "À suivre" and the libraries at once, then the latest additions of each
  /// shown library at once.
  fn load(&mut self, cx: &mut Context<Self>) {
    let prefs = Settings::get(cx).jellyfin.home.clone();
    let options = NextUpOptions {
      max_days: prefs.next_up_days,
      rewatching: prefs.next_up_rewatching,
    };
    let client = self.client.clone();
    let resume = runtime::spawn(cx, {
      let client = client.clone();
      async move { client.resume_items(ROW_LIMIT).await }
    });
    let next_up = runtime::spawn(cx, {
      let client = client.clone();
      async move { client.next_up(ROW_LIMIT, &options).await }
    });
    let views = runtime::spawn(cx, {
      let client = client.clone();
      async move { client.user_views().await }
    });
    cx.spawn(async move |this, cx| {
      let first = async {
        let resume = resume.await?;
        let next_up = next_up.await?;
        let views = views.await?;
        Some((resume, next_up, views))
      };
      let Some((resume, next_up, views)) = first.await else {
        return;
      };
      let (resume, next_up, views) = match (resume, next_up, views) {
        (Ok(r), Ok(n), Ok(v)) => (r.items, n.items, v.items),
        (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => {
          this.update(cx, |this, cx| this.failed(&e, cx)).ok();
          return;
        }
      };
      let libraries: Vec<BaseItem> = views
        .into_iter()
        .filter(|v| {
          v.collection_type.is_some_and(CollectionType::is_video)
            && !prefs.hidden_latest.contains(&v.id.to_string())
        })
        .collect();
      let Ok(latest) = this.update(cx, |_, cx| {
        libraries
          .iter()
          .map(|library| {
            let client = client.clone();
            let id = library.id.clone();
            runtime::spawn(cx, async move { client.latest(&id, ROW_LIMIT).await })
          })
          .collect::<Vec<_>>()
      }) else {
        return;
      };
      // The banner draws from both rows, even when they are hidden.
      let hero = if prefs.show_hero {
        hero_items(&resume, &next_up)
      } else {
        Vec::new()
      };
      let mut rows = Vec::new();
      if prefs.show_continue_watching {
        rows.push(Row {
          title: t("Continue watching", "Reprendre").into(),
          shape: Shape::Thumb,
          items: resume,
          // Playback resumes from the saved position.
          play: true,
        });
      }
      if prefs.show_next_up {
        rows.push(Row {
          title: t("Next up", "À suivre").into(),
          shape: Shape::Thumb,
          items: next_up,
          play: true,
        });
      }
      for (library, fetch) in libraries.into_iter().zip(latest) {
        match fetch.await {
          Some(Ok(items)) => rows.push(Row {
            title: tf!(
              "Latest in {}",
              "Derniers ajouts dans {}",
              library.name.unwrap_or_default()
            )
            .into(),
            shape: Shape::LargePoster,
            items,
            play: false,
          }),
          Some(Err(e)) => {
            this.update(cx, |this, cx| this.failed(&e, cx)).ok();
            return;
          }
          None => return,
        }
      }
      this
        .update(cx, |this, cx| {
          this.loading = false;
          // A reload keeps the slide shown when it is still there.
          let shown = this.hero.get(this.hero_index).map(|i| i.id.clone());
          this.hero_index = shown
            .and_then(|id| hero.iter().position(|i| i.id == id))
            .unwrap_or(0);
          this.hero_previous = None;
          this.hero = hero;
          this.rows = rows;
          this.start_rotation(cx);
          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  fn failed(&mut self, error: &Error, cx: &mut Context<Self>) {
    self.loading = false;
    report(&self.nav, error, cx);
    cx.notify();
  }

  fn start_rotation(&mut self, cx: &mut Context<Self>) {
    if self.hero.len() < 2 {
      self._rotation = None;
      return;
    }
    self._rotation = Some(cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor().timer(HERO_INTERVAL).await;
        let alive = this.update(cx, |this, cx| {
          if !this.hero_hovered && !this.hero.is_empty() {
            let next = (this.hero_index + 1) % this.hero.len();
            this.show_slide(next, cx);
          }
        });
        if alive.is_err() {
          break;
        }
      }
    }));
  }

  fn show_slide(&mut self, index: usize, cx: &mut Context<Self>) {
    if index != self.hero_index {
      self.hero_previous = Some(self.hero_index);
      self.hero_index = index;
      cx.notify();
    }
  }

  fn open(&self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
    self
      .nav
      .update(cx, |space, cx| space.open(route, window, cx))
      .ok();
  }

  fn render_hero(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
    let index = self.hero_index;
    let item = self.hero.get(index)?.clone();
    let width = f32::from(window.viewport_size().width);
    // Every slide's backdrop and logo are asked for on each frame, shown or not: they load
    // ahead of time, and their image state (the fade-in of `image_fill`) stays alive, so a
    // slide change never falls back to the BlurHash or swaps the title for the logo midway.
    let mut backdrops: Vec<_> = self
      .hero
      .iter()
      .map(|item| {
        let picture =
          backdrop_ref(item).map_or(Picture::Sizes(Vec::new()), |b| b.picture(&self.client));
        Some(image_fill(&picture, width, 16. / 9., px(0.), window, cx))
      })
      .collect();
    let mut logos: Vec<_> = self
      .hero
      .iter()
      .map(|item| logo_ref(item).and_then(|l| logo(&l.logo(&self.client), 420., 130., window, cx)))
      .collect();
    let backdrop = backdrops[index].take();
    let previous = self
      .hero_previous
      .filter(|&p| p != index)
      .and_then(|p| backdrops.get_mut(p)?.take());
    let logo = logos[index].take();
    let (title, meta) = hero_text(&item);
    let overview = item
      .overview
      .as_deref()
      .filter(|_| !is_spoiler(&item, cx))
      .map(one_paragraph);
    let play_id = item.id.clone();
    let details = route_for(&item);

    let dots = h_flex()
      .absolute()
      .right_8()
      .bottom(px(44.))
      .gap_2()
      .children((0..self.hero.len()).map(|i| {
        let active = i == index;
        div()
          .id(("hero-dot", i))
          .h(px(8.))
          .w(px(if active { 24. } else { 8. }))
          .rounded_full()
          .cursor_pointer()
          .bg(if active {
            theme::accent()
          } else {
            gpui::white().opacity(0.35)
          })
          .hover(|s| s.bg(gpui::white().opacity(0.7)))
          .on_click(cx.listener(move |this, _, _, cx| this.show_slide(i, cx)))
      }));

    let text = v_flex()
      .absolute()
      .left_8()
      .bottom(px(40.))
      .max_w(px(560.))
      .gap_3()
      .child(match logo {
        Some(logo) => logo,
        None => div()
          .text_3xl()
          .font_weight(FontWeight::BOLD)
          .line_clamp(2)
          .child(title)
          .into_any_element(),
      })
      .child(
        div()
          .text_sm()
          .font_weight(FontWeight::MEDIUM)
          .text_color(theme::accent_hover())
          .child(meta),
      )
      .children(overview.map(|text| {
        div()
          .text_sm()
          .line_clamp(3)
          .text_color(theme::foreground().opacity(0.8))
          .child(text)
      }))
      .child(
        h_flex()
          .pt_2()
          .gap_3()
          .child(
            Button::new("hero-play")
              .primary()
              .large()
              .icon(Icon::Play)
              .label(if resumable(&item) {
                t("Resume", "Reprendre")
              } else {
                t("Play", "Lire")
              })
              .on_click(cx.listener(move |this, _, window, cx| {
                this.open(Route::Play(play_id.clone()), window, cx)
              })),
          )
          .child(
            Button::new("hero-details")
              .large()
              .icon(Icon::Info)
              .label(t("Details", "Détails"))
              .on_click(
                cx.listener(move |this, _, window, cx| this.open(details.clone(), window, cx)),
              ),
          ),
      )
      // Each slide's text slides in from the left.
      .with_animation(
        ("hero-text", index),
        Animation::new(Duration::from_millis(350)).with_easing(ease_out_quint()),
        |el, t| el.opacity(t).left(px(32. - 16. * (1. - t))),
      );

    Some(
      div()
        .id("hero")
        .relative()
        .w_full()
        .h(px(HERO_HEIGHT))
        .flex_none()
        .overflow_hidden()
        .on_hover(cx.listener(|this, hovered: &bool, _, _| this.hero_hovered = *hovered))
        // The new backdrop fades in over the previous one.
        .children(previous.map(|el| div().absolute().inset_0().child(el)))
        .child(
          div()
            .absolute()
            .inset_0()
            .children(backdrop)
            .with_animation(
              ("hero-backdrop", index),
              Animation::new(HERO_FADE).with_easing(ease_out_quint()),
              |el, t| el.opacity(t),
            ),
        )
        // Darken the left for the text, and fade the bottom into the page.
        .child(div().absolute().inset_0().bg(linear_gradient(
          90.,
          linear_color_stop(theme::background().opacity(0.95), 0.),
          linear_color_stop(theme::background().opacity(0.), 0.7),
        )))
        .child(div().absolute().inset_0().bg(linear_gradient(
          180.,
          linear_color_stop(theme::background().opacity(0.), 0.55),
          linear_color_stop(theme::background(), 1.),
        )))
        .child(text)
        .child(dots)
        .into_any_element(),
    )
  }
}

/// Up to [`HERO_COUNT`] titles from "Reprendre" then "À suivre", one per series.
fn hero_items(resume: &[BaseItem], next_up: &[BaseItem]) -> Vec<BaseItem> {
  let mut seen = Vec::new();
  resume
    .iter()
    .chain(next_up)
    .filter(|item| {
      let key = item.series_id.clone().unwrap_or_else(|| item.id.clone());
      if seen.contains(&key) {
        false
      } else {
        seen.push(key);
        true
      }
    })
    .take(HERO_COUNT)
    .cloned()
    .collect()
}

/// The banner's title (the series for an episode) and the line under it.
fn hero_text(item: &BaseItem) -> (String, String) {
  let name = item.name.clone().unwrap_or_default();
  let left = item
    .user_data
    .as_ref()
    .filter(|u| u.playback_position_ticks > 0)
    .zip(item.run_time_ticks)
    .map(|(u, total)| {
      let minutes = (total - u.playback_position_ticks).max(0) / (60 * TICKS_PER_SECOND);
      tf!(" · {minutes} min left", " · {minutes} min restantes")
    })
    .unwrap_or_default();
  match item.kind {
    ItemKind::Episode => (
      item.series_name.clone().unwrap_or_default(),
      format!("{}{name}{left}", episode_number(item)),
    ),
    _ => {
      let year = item
        .production_year
        .map(|y| y.to_string())
        .unwrap_or_default();
      (name, format!("{year}{left}"))
    }
  }
}

fn skeleton_row(shape: Shape) -> impl IntoElement {
  v_flex()
    .gap_3()
    .child(
      Skeleton::new()
        .w(px(180.))
        .h(px(22.))
        .rounded(theme::RADIUS),
    )
    .child(
      h_flex()
        .gap_4()
        .children((0..6).map(move |_| skeleton_poster(shape.width(), shape.aspect()))),
    )
}

impl Render for HomeView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let hero = self.render_hero(window, cx);
    let mut rows = v_flex().px_8().pb_8().gap_8();
    if hero.is_none() {
      rows = rows.pt_8();
    }
    for (index, row) in self
      .rows
      .iter()
      .enumerate()
      .filter(|(_, r)| !r.items.is_empty())
    {
      let cards: Vec<_> = row
        .items
        .iter()
        .map(|item| {
          let route = if row.play {
            Route::Play(item.id.clone())
          } else {
            route_for(item)
          };
          let scope = format!("row{index}");
          card(
            &scope,
            item,
            row.shape,
            route,
            &self.nav,
            &self.client,
            window,
            cx,
          )
        })
        .collect();
      let row_view = scroll_row(format!("home-row-{index}"), cards, window, cx);
      rows = rows.child(
        v_flex()
          .gap_2()
          .child(section_title(row.title.clone()))
          .child(row_view),
      );
    }
    if !self.loading && self.hero.is_empty() && self.rows.iter().all(|r| r.items.is_empty()) {
      rows = rows.child(
        div()
          .text_color(theme::muted())
          .child(t("Nothing to show yet.", "Rien à afficher pour l'instant.")),
      );
    }
    let skeleton = v_flex()
      .p_8()
      .gap_8()
      .child(skeleton_row(Shape::Thumb))
      .child(skeleton_row(Shape::LargePoster))
      .into_any_element();
    let content = v_flex().children(hero).child(rows).into_any_element();
    let body = motion::crossfade(
      SharedString::from(format!("home-load-{}", cx.entity_id())),
      self.loading,
      skeleton,
      content,
      window,
      cx,
    );
    v_flex()
      .id("jellyfin-home")
      .size_full()
      .overflow_y_scrollbar()
      .child(body)
  }
}
