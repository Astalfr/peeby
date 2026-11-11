//! A channel's page: its header (`user`) with the follow button, its live when on air (GQL `Channel`), then its
//! past broadcasts (`videos?type=archive`, with the account's resume positions as progress bars
//! and their game chapters, each playing the VOD from there)
//! or its most viewed clips (`clips`), in two tabs; a third, "Planning", lists its streams to come
//! (GQL's `schedule`) when it has some, or the break it announced.

use std::collections::HashMap;

use gpui::prelude::*;
use gpui::{Context, FontWeight, Task, Window, div, px};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{h_flex, v_flex};
use twitch::catalog::{Catalog, Schedule};
use twitch::gql::{self, Channel};
use twitch::helix::{Clip, User, Video};
use ui::i18n::t;
use ui::images::Images;
use ui::picture;
use ui::theme;

use super::browse::load;
use super::{Nav, Route, cards, follow};

/// VODs and clips listed, as many as before with Helix.
const LIST: u32 = 40;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
  Videos,
  Clips,
  Schedule,
}

/// Segments of the schedule listed at most.
const SCHEDULE_ROWS: usize = 14;
/// A schedule row's box art, width and height (Twitch's 3:4).
const BOX_ART: (f32, f32) = (39., 52.);

struct Loaded {
  user: User,
  channel: Option<Channel>,
  videos: Vec<Video>,
  /// Where the viewer stopped in the VODs, seconds, by id.
  positions: HashMap<String, u32>,
  /// The VODs' game chapters, by id; the ones that kept one game are left out.
  chapters: HashMap<String, Vec<gql::Chapter>>,
  clips: Vec<Clip>,
  /// Its schedule, when it has one.
  schedule: Option<Schedule>,
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
    catalog: Catalog,
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
          let user = catalog.user(&login).await?;
          // The header's live and the lists are optional: their errors leave them out.
          let channel = client.channel(&login).await.ok();
          let videos = catalog.videos(&user.id, LIST).await.unwrap_or_default();
          let ids: Vec<String> = videos.iter().map(|v| v.id.clone()).collect();
          let positions = client.vod_positions(&ids).await.unwrap_or_else(|e| {
            tracing::warn!("Twitch VOD positions: {e}");
            HashMap::new()
          });
          let chapters = client.vod_chapters_many(&ids).await.unwrap_or_else(|e| {
            tracing::warn!("Twitch VOD chapters: {e}");
            HashMap::new()
          });
          let clips = catalog.clips(&user.id, LIST).await.unwrap_or_default();
          let schedule = catalog.schedule(&login).await.unwrap_or_else(|e| {
            tracing::warn!("Twitch schedule: {e}");
            None
          });
          Ok(Loaded {
            user,
            channel,
            videos,
            positions,
            chapters,
            clips,
            schedule,
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
      Tab::Schedule => "twitch-channel-tab-schedule",
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
          cards::loading().into_any_element(),
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
      Tab::Schedule => match &loaded.schedule {
        Some(schedule) => schedule_list(schedule, window, cx).into_any_element(),
        None => cards::note(t("No schedule.", "Aucun planning.")).into_any_element(),
      },
    };
    let has_schedule = loaded
      .schedule
      .as_ref()
      .is_some_and(|s| has_plans(s, now_millis()));
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
          )
          .child(follow::follow_button(
            "channel",
            &user.login,
            loaded.channel.as_ref().and_then(|c| c.followed),
          )),
      )
      .children(live)
      .child(
        h_flex()
          .gap_2()
          .child(self.tab_button(Tab::Videos, t("Videos", "Vidéos"), cx))
          .child(self.tab_button(Tab::Clips, t("Clips", "Clips"), cx))
          .when(has_schedule, |el| {
            el.child(self.tab_button(Tab::Schedule, t("Schedule", "Planning"), cx))
          }),
      )
      .child(list)
      .into_any_element()
  }
}

/// Now, epoch milliseconds.
fn now_millis() -> i64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map_or(0, |d| d.as_millis() as i64)
}

/// Whether `schedule` has something to show at `now`: a stream to come, or a break not over.
fn has_plans(schedule: &Schedule, now: i64) -> bool {
  !schedule.upcoming(now).is_empty() || vacation_until(schedule, now).is_some()
}

/// The end of the break `schedule` announces, if it is not over (`Some(None)`: open-ended).
fn vacation_until(schedule: &Schedule, now: i64) -> Option<Option<i64>> {
  let (_, end) = schedule.vacation?;
  end.is_none_or(|end| end > now).then_some(end)
}

/// The streams to come, soonest first, each its day and hours, its category's box art, its title
/// and category; a
/// cancelled one dimmed with "Cancelled". A break comes first.
fn schedule_list(
  schedule: &Schedule,
  window: &mut Window,
  cx: &mut gpui::App,
) -> impl IntoElement + use<> {
  let now = now_millis();
  let today = ui::calendar::today();
  let vacation = vacation_until(schedule, now).map(|end| {
    let text = match end {
      Some(end) => ui::tf!(
        "On a break until {}",
        "En pause jusqu'au {}",
        ui::calendar::day(end, today).to_lowercase()
      ),
      None => t("On a break", "En pause").to_owned(),
    };
    cards::note(text)
  });
  let rows = schedule
    .upcoming(now)
    .into_iter()
    .take(SCHEDULE_ROWS)
    .map(|segment| {
      let title = if segment.title.is_empty() {
        segment
          .category
          .clone()
          .unwrap_or_else(|| t("Stream", "Live").to_owned())
      } else {
        segment.title.clone()
      };
      let category = (!segment.title.is_empty())
        .then(|| segment.category.clone())
        .flatten();
      let live = segment.start <= now;
      // The category's box art (3:4), or its place kept.
      let art = div()
        .w(px(BOX_ART.0))
        .h(px(BOX_ART.1))
        .flex_none()
        .children(segment.box_art.as_deref().map(|art| {
          picture::image_fill(
            &cards::template(art),
            BOX_ART.0,
            BOX_ART.0 / BOX_ART.1,
            px(4.),
            window,
            cx,
          )
        }));
      h_flex()
        .gap_6()
        .py_3()
        .items_center()
        .border_b_1()
        .border_color(theme::border())
        .when(segment.cancelled, |el| el.opacity(0.5))
        .child(
          v_flex()
            .w(px(240.))
            .flex_none()
            .child(
              div()
                .font_weight(FontWeight::SEMIBOLD)
                .child(ui::calendar::day(segment.start, today)),
            )
            .child(
              div()
                .text_sm()
                .text_color(theme::muted())
                .child(ui::calendar::hours(segment.start, segment.end)),
            ),
        )
        .child(art)
        .child(
          v_flex()
            .min_w_0()
            .flex_1()
            .child(
              h_flex()
                .gap_2()
                .items_center()
                .child(div().min_w_0().truncate().child(title))
                .when(segment.cancelled, |el| {
                  el.child(
                    div()
                      .px_2()
                      .rounded_full()
                      .text_xs()
                      .bg(theme::surface_hover())
                      .child(t("Cancelled", "Annulé")),
                  )
                })
                .when(live && !segment.cancelled, |el| {
                  el.child(
                    div()
                      .px_2()
                      .rounded_full()
                      .text_xs()
                      .bg(theme::accent())
                      .text_color(theme::on_accent())
                      .child(t("Now", "En cours")),
                  )
                }),
            )
            .children(category.map(|c| div().text_sm().text_color(theme::muted()).child(c))),
        )
    })
    .collect::<Vec<_>>();
  v_flex().children(vacation).children(rows)
}
