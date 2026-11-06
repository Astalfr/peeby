//! Watch page: the player (autoplay), title, channel with its subscribe button and notification bell, like and dislike buttons (counts from
//! Return YouTube Dislike), description, comments and related videos. In fullscreen only the player is shown;
//! in theater mode (`Settings::theater`) the player spans the page above the two columns.
//!
//! The stream (VISIONOS HLS, ~0.2 s), the page metadata (`next`) and the votes load in
//! parallel; mpv starts as soon as the stream is known, so playback begins about 1 s after the
//! click.
//!
//! Playing a playlist, the page shows its [`Queue`] above the related videos. When the video
//! ends (or on Next/Previous), the queue picks a video and the app replaces this page with that
//! video's page, which shares the same queue.
//!
//! A live happening now has its chat ([`chat`]) in a column on the page's height, as on Twitch
//! (`ui::chat::watch_layout`); the related videos then go under the details.

use std::collections::HashMap;
use std::time::Instant;

use gpui::prelude::*;
use gpui::{
  AnyElement, App, Context, Entity, FontWeight, Global, ScrollHandle, SharedString, Subscription,
  Task, Window, div, px, relative,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::scroll::ScrollableElement;
use gpui_component::skeleton::Skeleton;
use gpui_component::tooltip::Tooltip;
use gpui_component::{Selectable, h_flex, v_flex};
use innertube::{Client, LikeStatus, TextLink, Votes, WatchPage};

use crate::settings::{Player, Settings};
use crate::shell::space::middle_click;
use crate::spaces::youtube::cards::{compact_card, playlist_row};
use crate::spaces::youtube::comments::{CommentSection, HoveredLink, Links, linked_text};
use crate::spaces::youtube::convert::{self, error_text};
use crate::spaces::youtube::history::{self, Reporter};
use crate::spaces::youtube::queue::{Queue, QueueEvent};
use crate::spaces::youtube::rating::{self, Rating};
use crate::spaces::youtube::{Nav, Route};
use crate::spaces::youtube::{bell, chat, prefetch, save, subscribe, tools};
use ui::assets::Icon;
use ui::card::skeleton_compact;
use ui::chat::{ChatPanel, watch_layout};
use ui::i18n::{Lang, lang, t};
use ui::images::Images;
use ui::picture::image_fill;
use ui::player::{PlayerEvent, PlayerView, Prefs};
use ui::{runtime, theme};

/// Width of the related videos column.
const RELATED_WIDTH: f32 = 400.;
/// Width of a thumbnail in the queue panel.
const QUEUE_THUMB: f32 = 100.;
/// Queue entries shown before and after the current one: a long playlist would otherwise
/// build thousands of rows on every frame.
const QUEUE_BEFORE: usize = 50;
const QUEUE_AFTER: usize = 150;
/// Lines of the description shown until it is expanded.
const DESCRIPTION_LINES: usize = 4;
/// The like button and the ratio bar's likes: the blue of YouTube's thumb (dark theme).
const LIKE_BLUE: u32 = 0x3ea6ff;
/// The dislike button and the ratio bar's dislikes: YouTube red.
const DISLIKE_RED: u32 = 0xff0000;
/// Theater mode: height left under the player for the start of the title, and the player's
/// least height.
const THEATER_BELOW: f32 = 120.;
const THEATER_MIN: f32 = 240.;

/// Return YouTube Dislike results for the session: its API allows 100 requests a minute and
/// 10,000 a day per client.
#[derive(Default)]
struct VotesCache(HashMap<String, Votes>);

impl Global for VotesCache {}

pub struct WatchView {
  client: Client,
  video_id: String,
  nav: Nav,
  player: Entity<PlayerView>,
  page: Option<WatchPage>,
  /// The whole description is shown ("Show more").
  description_open: bool,
  /// The description's link under the pointer.
  hovered_link: HoveredLink,
  votes: Option<Votes>,
  rating: Rating,
  comments: Entity<CommentSection>,
  scroll: ScrollHandle,
  /// The playlist being played, if any.
  queue: Option<Entity<Queue>>,
  queue_scroll: ScrollHandle,
  /// Row of the queue panel to scroll to the top, once the panel has been laid out.
  queue_scroll_to: Option<usize>,
  /// A live's chat, and the task polling it.
  chat: Option<(Entity<ChatPanel>, Task<()>)>,
  /// Where playback starts: where it stopped last time ([`history::start_at`]).
  start: Option<f64>,
  /// Reports the playback to the account's history.
  history: Reporter,
  _subscriptions: Vec<Subscription>,
}

impl WatchView {
  /// The watch page of `video_id`; `queue` when it plays a playlist (its current video).
  pub fn new(
    client: Client,
    video_id: String,
    queue: Option<Entity<Queue>>,
    nav: Nav,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let settings = Settings::get(cx);
    let prefs = Prefs {
      subtitle_language: Some(settings.language.clone()),
      ..Player::prefs(settings)
    };
    let theater = settings.theater;
    let player = cx.new(|cx| PlayerView::new(prefs, theater, cx));
    let focus = player.read(cx).focus_handle().clone();
    window.focus(&focus, cx);
    let votes = cx
      .try_global::<VotesCache>()
      .and_then(|cache| cache.0.get(&video_id).copied());
    let start = history::start_at(&video_id, cx);
    let mut subscriptions = vec![
      cx.subscribe(&player, |this, player, event: &PlayerEvent, cx| {
        this.history.on_event(event, &player, cx);
        match *event {
          // Kept in the settings, so the next videos open the same way.
          PlayerEvent::Theater => {
            Settings::update(cx, |s| s.theater = !s.theater);
            let theater = Settings::get(cx).theater;
            player.update(cx, |player, cx| player.set_theater(theater, cx));
            cx.notify();
          }
          PlayerEvent::Sound { volume, muted } => Player::save_sound(volume, muted, cx),
          PlayerEvent::Ended | PlayerEvent::Next => {
            if let Some(queue) = &this.queue {
              queue.update(cx, |q, cx| q.next(cx));
            }
          }
          PlayerEvent::Previous => {
            if let Some(queue) = &this.queue {
              queue.update(cx, |q, cx| q.previous(cx));
            }
          }
          // Reported by `history`; the embedded player has no close button.
          PlayerEvent::Started
          | PlayerEvent::Paused(_)
          | PlayerEvent::Seeked
          | PlayerEvent::TracksChanged { .. }
          | PlayerEvent::Rewound { .. }
          | PlayerEvent::Close => {}
        }
      }),
    ];
    let queue_scroll = ScrollHandle::new();
    if let Some(queue) = &queue {
      // The queue chose another video: this page makes way for that video's page.
      let nav = nav.clone();
      subscriptions.push(cx.subscribe_in(
        queue,
        window,
        move |_, queue, QueueEvent::Play, window, cx| {
          let (queue, tab) = (queue.clone(), nav.tab);
          nav
            .update(cx, |app, cx| app.play_queue(tab, queue, true, window, cx))
            .ok();
        },
      ));
      subscriptions.push(cx.observe(queue, |this, _, cx| {
        this.sync_queue_controls(cx);
        cx.notify();
      }));
      queue.update(cx, |q, cx| q.started(cx));
    }
    // The current video near the top of the panel, under the one before it.
    let queue_scroll_to = queue.as_ref().map(|queue| {
      let current = queue.read(cx).place();
      current.min(QUEUE_BEFORE).saturating_sub(1)
    });
    // The page goes (replaced, back, closed): the last watch-time ping.
    cx.on_release(|this: &mut Self, cx| {
      let player = this.player.clone();
      this.history.close(&player, cx);
    })
    .detach();
    let history = Reporter::new(client.clone(), video_id.clone());
    let scroll = ScrollHandle::new();
    let comments = cx.new(|cx| {
      CommentSection::new(
        client.clone(),
        video_id.clone(),
        nav.clone(),
        player.clone(),
        scroll.clone(),
        true,
        cx,
      )
    });
    let view = Self {
      client,
      video_id,
      nav,
      player,
      page: None,
      votes,
      rating: Rating::default(),
      comments,
      scroll,
      description_open: false,
      hovered_link: None,
      queue,
      queue_scroll,
      queue_scroll_to,
      chat: None,
      start,
      history,
      _subscriptions: subscriptions,
    };
    view.sync_queue_controls(cx);
    view.load_stream(cx);
    view.report_history(cx);
    view.load_page(cx);
    if view.votes.is_none() {
      view.load_votes(cx);
    }
    view
  }

  /// The player's previous and next buttons follow the queue.
  fn sync_queue_controls(&self, cx: &mut Context<Self>) {
    let controls = self.queue.as_ref().map(|queue| {
      let queue = queue.read(cx);
      (queue.has_previous(), queue.has_next())
    });
    self
      .player
      .update(cx, |player, cx| player.set_queue(controls, cx));
  }

  /// The playlist next to the player: title, place, shuffle and repeat, and its videos in
  /// play order, the current one highlighted. A click plays that video.
  fn queue_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
    let entity = self.queue.clone()?;
    // GPUI scrolls to a row from the list's bounds of the last frame: wait until it has some.
    if let Some(row) = self.queue_scroll_to {
      if self.queue_scroll.bounds().size.height > px(0.) {
        self.queue_scroll.scroll_to_top_of_item(row);
        self.queue_scroll_to = None;
      } else {
        window.request_animation_frame();
      }
    }
    let queue = entity.read(cx);
    let (title, position, total) = (queue.title.clone(), queue.position(), queue.order().len());
    let playlist_id = queue.playlist_id.clone();
    let (shuffle, repeat, more) = (queue.shuffle(), queue.repeat(), queue.has_more());
    // Once the video playing was removed, `current` is the one that followed it, not highlighted.
    let (current, playing) = (queue.place(), queue.playing_place());
    let start = current.saturating_sub(QUEUE_BEFORE);
    let end = (current + QUEUE_AFTER + 1).min(total);
    let entries: Vec<_> = queue.order()[start..end]
      .iter()
      .enumerate()
      .map(|(offset, &index)| (start + offset, index, queue.items()[index].clone()))
      .collect();
    let rows: Vec<_> = entries
      .into_iter()
      .map(|(place, index, video)| {
        let queue = entity.clone();
        // The playing video too: it plays on, out of the queue.
        let remove = video
          .set_video_id
          .clone()
          .map(|entry| save::removal(&playlist_id, &title, &entry, &video.id, cx));
        playlist_row(
          &format!("q-{place}-{}", video.id),
          place + 1,
          &video,
          QUEUE_THUMB,
          Some(place) == playing,
          move |_, cx| queue.update(cx, |q, cx| q.jump(index, cx)),
          remove,
          window,
          cx,
        )
      })
      .collect();
    let toggle = |id: &'static str, icon: Icon, tooltip: &'static str, on: bool| {
      Button::new(id)
        .ghost()
        .icon(icon)
        .tooltip(tooltip)
        .selected(on)
    };
    Some(
      v_flex()
        .w_full()
        .rounded(theme::RADIUS_LG)
        .border_1()
        .border_color(theme::border())
        .bg(theme::surface().opacity(0.5))
        .overflow_hidden()
        .child(
          h_flex()
            .px_4()
            .py_3()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(theme::border())
            .child(
              v_flex()
                .flex_1()
                .min_w_0()
                .child(
                  div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(title),
                )
                .child(div().text_xs().text_color(theme::muted()).child(format!(
                  "{position} / {total}{}",
                  if more { "+" } else { "" }
                ))),
            )
            .child(
              toggle(
                "queue-shuffle",
                Icon::Shuffle,
                t("Shuffle", "Lecture aléatoire"),
                shuffle,
              )
              .on_click(cx.listener(|this, _, _, cx| {
                let Some(queue) = this.queue.clone() else {
                  return;
                };
                queue.update(cx, |q, cx| q.toggle_shuffle(cx));
                // The order changed: bring the current video back into view.
                let current = queue.read(cx).place();
                this.queue_scroll_to = Some(current.min(QUEUE_BEFORE).saturating_sub(1));
                cx.notify();
              })),
            )
            .child(
              toggle(
                "queue-repeat",
                Icon::Repeat,
                t("Repeat the playlist", "Répéter la playlist"),
                repeat,
              )
              .on_click({
                let queue = entity.clone();
                move |_, _, cx| queue.update(cx, |q, cx| q.toggle_repeat(cx))
              }),
            ),
        )
        .child(
          div()
            .relative()
            .child(
              v_flex()
                .id("queue-list")
                .max_h(px(420.))
                .p_1()
                .overflow_y_scroll()
                .track_scroll(&self.queue_scroll)
                // GPUI hands the wheel to every scrollable ancestor too: keep it from scrolling the
                // page while the list has something to scroll (its own listener has already run).
                .on_scroll({
                  let scroll = self.queue_scroll.clone();
                  move |_, _, cx| {
                    if scroll.max_offset().y > px(0.) {
                      cx.stop_propagation();
                    }
                  }
                })
                .children(rows),
            )
            // After the list: children paint in order, and a hovered row would cover it.
            .vertical_scrollbar(&self.queue_scroll),
        )
        .into_any_element(),
    )
  }

  /// The page's player.
  pub fn player(&self) -> Entity<PlayerView> {
    self.player.clone()
  }

  /// Pauses the player while another page covers this one.
  pub fn set_covered(&mut self, covered: bool, cx: &mut Context<Self>) {
    self
      .player
      .update(cx, |player, _| player.set_covered(covered));
  }

  fn load_stream(&self, cx: &mut Context<Self>) {
    load_stream(&self.client, &self.video_id, self.start, &self.player, cx);
  }

  /// Asks for the stats URLs (signed in), then pings every [`history::PING_EVERY`] while the
  /// page lives.
  fn report_history(&self, cx: &mut Context<Self>) {
    history::report(
      &self.history,
      |this| (&mut this.history, this.player.clone()),
      cx,
    );
  }

  /// Follows the live's chat from `continuation`.
  fn start_chat(&mut self, continuation: String, cx: &mut Context<Self>) {
    if self.chat.is_some() {
      return;
    }
    let panel = cx.new(ChatPanel::new);
    let task = chat::start(self.client.clone(), continuation, &panel, cx);
    self.chat = Some((panel, task));
  }

  fn load_page(&self, cx: &mut Context<Self>) {
    let (client, id) = (self.client.clone(), self.video_id.clone());
    let fetch = runtime::spawn(cx, async move { client.watch(&id).await });
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |this, cx| {
          match result {
            Some(Ok(page)) => {
              let title = SharedString::from(page.title.clone());
              this.nav.set_title(cx.entity_id(), title.clone(), cx);
              let chapters = convert::chapters(&page.chapters);
              let author = convert::author(&page);
              this.player.update(cx, |player, cx| {
                player.set_title(title, cx);
                player.set_author(author, cx);
                if !chapters.is_empty() {
                  player.set_chapters(chapters, cx);
                }
              });
              this.rating = Rating::of(&page);
              if let Some(continuation) = page.live_chat.clone() {
                this.start_chat(continuation, cx);
              }
              let token = page.comments_continuation.clone();
              this.comments.update(cx, |c, cx| c.start(token, cx));
              this.page = Some(page);
            }
            Some(Err(e)) => tracing::warn!("watch page: {e}"),
            None => {}
          }
          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  fn load_votes(&self, cx: &mut Context<Self>) {
    let (client, id) = (self.client.clone(), self.video_id.clone());
    let fetch = runtime::spawn(cx, async move { client.votes(&id).await });
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |this, cx| match result {
          Some(Ok(votes)) => {
            cx.default_global::<VotesCache>()
              .0
              .insert(this.video_id.clone(), votes);
            this.votes = Some(votes);
            cx.notify();
          }
          Some(Err(e)) => tracing::warn!("Return YouTube Dislike: {e}"),
          None => {}
        })
        .ok();
    })
    .detach();
  }

  fn rate(&mut self, clicked: LikeStatus, window: &mut Window, cx: &mut Context<Self>) {
    if self.page.is_none() {
      return;
    }
    let (client, id) = (self.client.clone(), self.video_id.clone());
    rating::rate(
      self,
      |this| &mut this.rating,
      &client,
      &id,
      clicked,
      window,
      cx,
    );
  }

  /// The like and dislike buttons side by side on a pill, with the Return YouTube Dislike
  /// counts once known: the ratio bar under them and, right of both, the attribution RYD
  /// requires. The counts follow the user's clicks.
  fn rating_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let initial = self
      .page
      .as_ref()
      .map(|p| p.like_status)
      .unwrap_or_default();
    let current = self.rating.status;
    let votes = self.votes.map(|votes| {
      let shift = |n: u64, status| {
        (n + u64::from(current == status)).saturating_sub(u64::from(initial == status))
      };
      Votes {
        likes: shift(votes.likes, LikeStatus::Like),
        dislikes: shift(votes.dislikes, LikeStatus::Dislike),
      }
    });
    let button = |id: &'static str,
                  icon: Icon,
                  status: LikeStatus,
                  color: u32,
                  count: Option<u64>,
                  label: &'static str,
                  tooltip: &'static str,
                  cx: &mut Context<Self>| {
      h_flex()
        .id(id)
        .h_full()
        .px_3()
        .gap_2()
        .items_center()
        .cursor_pointer()
        // The button takes its thumb's color (YouTube's blue or red) when it is the user's
        // rating and on hover, over a soft wash of it.
        .hover(move |s| {
          let color = gpui::Hsla::from(gpui::rgb(color));
          s.bg(color.opacity(0.14)).text_color(color)
        })
        .when(current == status, |el| el.text_color(gpui::rgb(color)))
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .on_click(cx.listener(move |this, _, window, cx| this.rate(status, window, cx)))
        .child(gpui_component::Icon::new(icon).size(px(16.)))
        .child(match count {
          Some(count) => format!("{} {label}", compact_number(count)),
          None => label.to_owned(),
        })
    };
    let (like_tip, dislike_tip) = match current {
      LikeStatus::Like => (
        t("Remove like", "Je n'aime plus"),
        t("Dislike", "Je n'aime pas"),
      ),
      LikeStatus::Dislike => (
        t("Like", "J'aime"),
        t("Remove dislike", "Retirer « Je n'aime pas »"),
      ),
      LikeStatus::Indifferent => (t("Like", "J'aime"), t("Dislike", "Je n'aime pas")),
    };
    let pill = h_flex()
      .h(px(36.))
      .rounded_full()
      .overflow_hidden()
      .bg(theme::surface())
      .text_sm()
      .font_weight(FontWeight::MEDIUM)
      .items_center()
      .child(
        button(
          "like",
          Icon::ThumbsUp,
          LikeStatus::Like,
          LIKE_BLUE,
          votes.map(|v| v.likes),
          "Likes",
          like_tip,
          cx,
        )
        // GPUI clips children to the pill's rectangle, not its corners: each half rounds its
        // outer side so its hover follows the pill.
        .rounded_l_full(),
      )
      .child(div().w(px(1.)).h(px(20.)).bg(theme::border()))
      .child(
        button(
          "dislike",
          Icon::ThumbsDown,
          LikeStatus::Dislike,
          DISLIKE_RED,
          votes.map(|v| v.dislikes),
          "Dislikes",
          dislike_tip,
          cx,
        )
        .rounded_r_full(),
      );
    let ratio = votes.map(|votes| {
      let total = votes.likes + votes.dislikes;
      let ratio = if total > 0 {
        votes.likes as f32 / total as f32
      } else {
        1.0
      };
      // Likes in YouTube's blue thumb over the dislikes in its red, whatever the accent.
      div()
        .mx_3()
        .h(px(3.))
        .rounded_full()
        .bg(gpui::rgb(DISLIKE_RED))
        .child(
          div()
            .h_full()
            .w(relative(ratio))
            .rounded_full()
            .bg(gpui::rgb(LIKE_BLUE)),
        )
    });
    // The attribution RYD requires: an "i" right of the pill, its tooltip naming the source, a
    // click opening its site.
    let attribution = votes.map(|_| {
      div()
        .id("ryd")
        .flex_none()
        .p_1()
        .rounded_full()
        .text_color(theme::subtle())
        .cursor_pointer()
        .hover(|s| s.text_color(theme::foreground()))
        .tooltip(|window, cx| {
          Tooltip::new(t(
            "Dislikes from Return YouTube Dislike (returnyoutubedislike.com)",
            "Dislikes fournis par Return YouTube Dislike (returnyoutubedislike.com)",
          ))
          .build(window, cx)
        })
        .on_click(|_, _, cx| cx.open_url("https://returnyoutubedislike.com"))
        .child(gpui_component::Icon::new(Icon::Info).size(px(16.)))
    });
    // The ratio bar takes the pill's width: both sit in a column as wide as the pill.
    h_flex()
      .items_center()
      .gap_1()
      .child(v_flex().gap_1p5().child(pill).children(ratio))
      .children(attribution)
  }

  fn details(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let Some(page) = &self.page else {
      return skeleton_details();
    };
    let avatar = (!page.channel_avatar.is_empty()).then(|| {
      div().size(px(40.)).flex_none().child(image_fill(
        &convert::picture(&page.channel_avatar),
        40.,
        1.,
        px(20.),
        window,
        cx,
      ))
    });
    let channel = page.channel_id.clone().map(|id| Route::Channel {
      id,
      name: page.channel.clone().unwrap_or_default(),
    });
    let stats = [page.views.clone(), page.date.clone()]
      .into_iter()
      .flatten()
      .collect::<Vec<_>>()
      .join(" · ");
    // `line_clamp` limits wrapped lines, not explicit line breaks: cut those first.
    let cut = page
      .description
      .match_indices('\n')
      .nth(DESCRIPTION_LINES - 1)
      .map_or(page.description.len(), |(i, _)| i);
    // Folded, a description that could wrap past its lines gets the toggle too.
    let long = cut < page.description.len() || page.description.len() > 300;
    let links = Links::new(
      self.video_id.clone(),
      self.nav.clone(),
      self.player.clone(),
      Some(self.scroll.clone()),
      cx.weak_entity(),
      self.hovered_link.clone(),
      |this| &mut this.hovered_link,
    );
    let description = if self.description_open {
      linked_text(
        "description",
        &page.description,
        &page.description_links,
        &links,
      )
      .into_any_element()
    } else {
      let shown: Vec<TextLink> = page
        .description_links
        .iter()
        .filter(|l| l.range.end <= cut)
        .cloned()
        .collect();
      div()
        .line_clamp(DESCRIPTION_LINES)
        .child(linked_text(
          "description",
          &page.description[..cut],
          &shown,
          &links,
        ))
        .into_any_element()
    };
    let toggle = long.then(|| {
      h_flex().child(
        Button::new("description-toggle")
          .ghost()
          .compact()
          .label(if self.description_open {
            t("Show less", "Afficher moins")
          } else {
            t("Show more", "Afficher plus")
          })
          .on_click(cx.listener(|this, _, _, cx| {
            this.description_open = !this.description_open;
            cx.notify();
          })),
      )
    });
    v_flex()
      .gap_4()
      .child(
        div()
          .text_xl()
          .font_weight(FontWeight::BOLD)
          .child(page.title.clone()),
      )
      .child(
        h_flex()
          .gap_3()
          .items_start()
          .child(
            h_flex()
              .id("channel")
              .h(px(40.))
              .group("channel")
              .gap_3()
              .items_center()
              .when_some(channel, |el, route| {
                let nav = self.nav.clone();
                el.cursor_pointer()
                  .on_aux_click(middle_click({
                    let (nav, route) = (nav.clone(), route.clone());
                    move |cx| nav.open_in_new_tab(route.clone(), None, cx)
                  }))
                  .on_click(move |_, window, cx| nav.open(route.clone(), window, cx))
              })
              .children(avatar)
              .child(
                v_flex()
                  .child(
                    div()
                      .font_weight(FontWeight::SEMIBOLD)
                      .group_hover("channel", |s| s.text_color(theme::accent_hover()))
                      .child(page.channel.clone().unwrap_or_default()),
                  )
                  .child(
                    div()
                      .text_sm()
                      .text_color(theme::muted())
                      .child(page.subscribers.clone().unwrap_or_default()),
                  ),
              ),
          )
          .children(
            page
              .channel_id
              .as_deref()
              .zip(page.subscription.as_ref())
              .map(|(channel_id, subscription)| {
                let key = format!("watch-{}", self.video_id);
                let subscribed = subscribe::is_subscribed(channel_id, subscription, cx);
                h_flex()
                  .h(px(40.))
                  .gap_1()
                  .items_center()
                  .child(subscribe::subscribe_button(
                    &key,
                    channel_id,
                    page.channel.as_deref().unwrap_or_default(),
                    subscription,
                    false,
                    cx,
                  ))
                  .children(page.notifications.as_ref().filter(|_| subscribed).map(
                    |notifications| bell::bell_button(&key, channel_id, notifications, false, cx),
                  ))
              }),
          )
          .child(div().flex_1())
          // The votes column also holds the ratio bar and the attribution: centered on it, the
          // channel would sit far below the title. Everything aligns on the 40 px avatar band
          // instead, the 36 px pill 2 px down.
          .child(
            h_flex()
              .mt(px(2.))
              .gap_3()
              .items_start()
              .child(self.rating_bar(cx))
              .child(
                h_flex()
                  .h(px(36.))
                  .items_center()
                  .child(save::watch_menu_button(&self.video_id)),
              ),
          ),
      )
      .child(
        v_flex()
          .px_4()
          .py_3()
          .gap_1()
          .rounded(theme::RADIUS_LG)
          .bg(theme::surface())
          .border_1()
          .border_color(theme::border())
          .text_sm()
          .child(div().font_weight(FontWeight::SEMIBOLD).child(stats))
          .child(description)
          .children(toggle),
      )
  }
}

/// Starts `video_id` in `player` at `start`: from the hover prefetch when it has the stream, else
/// from VISIONOS (~0.2 s), else through the bundled yt-dlp.
pub(super) fn load_stream(
  client: &Client,
  video_id: &str,
  start: Option<f64>,
  player: &Entity<PlayerView>,
  cx: &mut App,
) {
  let clicked = Instant::now();
  if let Some(stream) = prefetch::take(video_id, cx) {
    tracing::info!(
      "stream from hover prefetch: {} {}",
      stream.resolution,
      stream.codecs
    );
    let proxy = stream.live.as_ref().and_then(|l| l.proxy.clone());
    let mut source = convert::source(stream, &Settings::get(cx).player);
    resume(&mut source, start);
    player.update(cx, |player, cx| player.start(source, clicked, cx));
    settle_window(proxy, player, cx);
    return;
  }
  let max_height = Settings::get(cx).max_height;
  let player_settings = Settings::get(cx).player.clone();
  let (client, id) = (client.clone(), video_id.to_owned());
  let fetch = runtime::spawn(cx, {
    let client = client.clone();
    async move { client.stream(&id, max_height).await }
  });
  let player = player.downgrade();
  let video_id = video_id.to_owned();
  cx.spawn(async move |cx| {
    let error = match fetch.await {
      Some(Ok(stream)) => {
        tracing::info!(
          "stream after {:.2} s: {} {}",
          clicked.elapsed().as_secs_f64(),
          stream.resolution,
          stream.codecs
        );
        let proxy = stream.live.as_ref().and_then(|l| l.proxy.clone());
        let mut source = convert::source(stream, &player_settings);
        resume(&mut source, start);
        if let Some(player) = player.upgrade() {
          cx.update(|cx| {
            player.update(cx, |player, cx| player.start(source, clicked, cx));
            settle_window(proxy, &player, cx);
          });
        }
        return;
      }
      Some(Err(e)) => e,
      None => return,
    };
    // VISIONOS is an anonymous visitor: it refuses a members-only video even to a member. The
    // WEB player, with the session, tells whether the account may watch it (then yt-dlp, with
    // the cookies, plays it); otherwise say so rather than let yt-dlp fail after a few seconds.
    let error = match error {
      innertube::Error::Unplayable { ref status, .. } if status == "UNPLAYABLE" => {
        let id = video_id.clone();
        let check =
          cx.update(|cx| runtime::spawn(cx, async move { client.check_members_only(&id).await }));
        match check.await {
          Some(Err(members @ innertube::Error::MembersOnly { .. })) => members,
          _ => error,
        }
      }
      error => error,
    };
    // Network down: yt-dlp would fail too, and a members-only video the account cannot watch
    // stays refused. Anything else (age restriction, a member's video, client blocked,
    // unexpected response): let the bundled yt-dlp try (ADR 0005).
    let fallback = if matches!(
      error,
      innertube::Error::Http(_) | innertube::Error::MembersOnly { .. }
    ) {
      None
    } else {
      tracing::info!("VISIONOS failed ({error}), trying yt-dlp");
      cx.background_executor()
        .spawn(async move { tools::ytdl_options(&video_id) })
        .await
    };
    player
      .update(cx, |player, cx| match fallback {
        Some(mut options) => {
          player_settings.apply(&mut options);
          // A card's resume is never a live's.
          if let Some(start) = start {
            options.start_seconds = start;
          }
          player.start_with(options, clicked, cx)
        }
        None => {
          tracing::warn!("stream: {error}");
          player.fail(error_text(&error).into(), cx);
        }
      })
      .ok();
  })
  .detach();
}

/// A proxied live's window once its proxy has looked for the first segment kept (a 24/7 live
/// keeps less than YouTube's DVR length): the seek bar stops there.
fn settle_window(proxy: Option<innertube::LiveProxy>, player: &Entity<PlayerView>, cx: &mut App) {
  let Some(proxy) = proxy else {
    return;
  };
  let announced = proxy.window();
  let settled = runtime::spawn(cx, async move { proxy.settled().await });
  let player = player.downgrade();
  cx.spawn(async move |cx| {
    if let Some(window) = settled.await.filter(|w| *w < announced) {
      player
        .update(cx, |player, cx| player.set_dvr_window(window, cx))
        .ok();
    }
  })
  .detach();
}

/// Starts a video at `start` (a live keeps its own start, near its edge).
fn resume(source: &mut ui::player::Source, start: Option<f64>) {
  if let Some(start) = start.filter(|_| !source.live) {
    source.options.start_seconds = start;
  }
}

fn skeleton_details() -> gpui::Div {
  v_flex()
    .gap_4()
    .child(Skeleton::new().w(relative(0.7)).h(px(26.)))
    .child(
      h_flex()
        .gap_3()
        .items_center()
        .child(Skeleton::new().size(px(40.)).rounded_full())
        .child(
          v_flex()
            .gap_2()
            .child(Skeleton::new().w(px(160.)).h(px(16.)))
            .child(Skeleton::new().w(px(100.)).h(px(12.))),
        ),
    )
    .child(
      Skeleton::new()
        .w_full()
        .h(px(96.))
        .rounded(theme::RADIUS_LG),
    )
}

/// A short count in the interface's language ([`compact_number_in`]).
fn compact_number(n: u64) -> String {
  compact_number_in(n, lang())
}

/// `1.2K`, `3.4M`, `1.2B` in English; `1,2 k`, `3,4 M`, `1,2 Md` in French.
fn compact_number_in(n: u64, lang: Lang) -> String {
  let short = |value: f64, en: &str, fr: &str| {
    let text = if value < 10.0 {
      format!("{value:.1}")
    } else {
      format!("{value:.0}")
    };
    let text = text.trim_end_matches(".0");
    match lang {
      Lang::En => format!("{text}{en}"),
      Lang::Fr => format!("{} {fr}", text.replace('.', ",")),
    }
  };
  match n {
    0..1_000 => n.to_string(),
    1_000..1_000_000 => short(n as f64 / 1e3, "K", "k"),
    1_000_000..1_000_000_000 => short(n as f64 / 1e6, "M", "M"),
    _ => short(n as f64 / 1e9, "B", "Md"),
  }
}

impl Render for WatchView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    if window.is_fullscreen() {
      return div()
        .id("watch")
        .size_full()
        .child(self.player.clone())
        .into_any_element();
    }
    let related: Vec<_> = match &self.page {
      Some(page) => page
        .related
        .iter()
        .map(|v| compact_card(v, &self.nav, window, cx))
        .collect(),
      None => (0..8)
        .map(|_| skeleton_compact().into_any_element())
        .collect(),
    };
    let details = self.details(window, cx);
    let comments = self.comments.clone();
    let queue = self.queue_panel(window, cx);
    let theater = Settings::get(cx).theater;
    if let Some((panel, _)) = &self.chat {
      let below = v_flex()
        .gap_5()
        .child(details)
        .child(comments)
        .children(queue)
        .child(
          v_flex()
            .max_w(px(RELATED_WIDTH * 2.))
            .gap_2()
            .children(related),
        );
      return watch_layout(
        self.player.clone().into_any_element(),
        below.into_any_element(),
        Some(panel.clone().into_any_element()),
        theater,
      );
    }
    let player = if theater {
      // Across the whole page on black, 16:9 but short enough to leave the title in view;
      // mpv letterboxes the picture inside.
      let viewport = window.viewport_size();
      let width = viewport.width - theme::sidebar_width(Settings::get(cx).sidebar_collapsed);
      let room = viewport.height - theme::TITLE_BAR_HEIGHT - px(THEATER_BELOW);
      div()
        .w_full()
        .flex_none()
        .h((width * (9. / 16.)).min(room.max(px(THEATER_MIN))))
        // The content panel's rounded corner clips it there, a rectangle would cross it.
        .rounded_tl(theme::PANEL_INNER_RADIUS)
        .bg(gpui::black())
        .child(self.player.clone())
    } else {
      // 16:9 box: GPUI has no aspect-ratio style, so the height follows the width
      // through padding-top as a share of the width.
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
    let page = v_flex()
      .id("watch")
      .size_full()
      .overflow_y_scroll()
      .track_scroll(&self.scroll)
      .on_scroll(cx.listener(|this, _, _, cx| {
        this
          .comments
          .update(cx, |c, cx| c.load_more_if_near_end(cx));
        cx.notify();
      }))
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
              .child(details)
              .child(comments),
          )
          .child(
            v_flex()
              .w(px(RELATED_WIDTH))
              .flex_none()
              .gap_2()
              .children(queue.map(|panel| div().pb_2().child(panel)))
              .children(related),
          ),
      );
    div()
      .size_full()
      .relative()
      .child(page)
      .vertical_scrollbar(&self.scroll)
      .into_any_element()
  }
}

#[cfg(test)]
mod tests {
  use super::{compact_number, compact_number_in};
  use ui::i18n::Lang;

  #[test]
  fn compact_numbers() {
    assert_eq!(compact_number(999), "999");
    assert_eq!(compact_number(1_234), "1.2K");
    assert_eq!(compact_number(19_491_081), "19M");
    assert_eq!(compact_number(521_536), "522K");
    assert_eq!(compact_number(2_000_000), "2M");
    assert_eq!(compact_number(3_400_000_000), "3.4B");
  }

  #[test]
  fn compact_numbers_in_french() {
    assert_eq!(compact_number_in(999, Lang::Fr), "999");
    assert_eq!(compact_number_in(1_234, Lang::Fr), "1,2 k");
    assert_eq!(compact_number_in(19_491_081, Lang::Fr), "19 M");
    assert_eq!(compact_number_in(521_536, Lang::Fr), "522 k");
    assert_eq!(compact_number_in(2_000_000, Lang::Fr), "2 M");
    assert_eq!(compact_number_in(3_400_000_000, Lang::Fr), "3,4 Md");
  }
}
