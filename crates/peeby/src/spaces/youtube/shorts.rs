//! Shorts page: YouTube's Shorts one at a time, as on youtube.com/shorts. The Short fills the
//! page's height in a 9:16 player, its title and channel under it; on its right, the like,
//! dislike, comments, save and "watch as a video" buttons; at the page's right edge, the
//! previous and next arrows. The comments open in a panel beside the Short and stay open from
//! one Short to the next.
//!
//! The sequence is YouTube's (`Client::shorts`, then `Client::shorts_more` as it runs out): an
//! endless feed, personalized when signed in. Each Short is a [`ShortView`] with its own player,
//! made when it shows: the previous one goes, with its last history ping. The next Short's
//! stream is resolved ahead ([`prefetch::warm`]), so it starts at once. A Short loops at its
//! end, as on YouTube.
//!
//! The wheel, the up and down arrows (before the player, which would set the volume) and the
//! arrow buttons move through the Shorts. The move slides, as on YouTube: the Short leaving goes
//! off the page (up for the next one, down for the previous one), paused, while the new one comes
//! in from the other side and already plays.

use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
  Animation, AnimationExt, AnyElement, App, Context, Entity, EventEmitter, FontWeight,
  KeyDownEvent, Pixels, ScrollEvent, ScrollHandle, SharedString, Subscription, Window, div,
  ease_out_quint, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::scroll::ScrollableElement;
use gpui_component::skeleton::Skeleton;
use gpui_component::tooltip::Tooltip;
use gpui_component::{Disableable, IconName, h_flex, v_flex};
use innertube::{Client, LikeStatus, ShortsPage, WatchPage};

use crate::settings::{Player, Settings};
use crate::shell::space::middle_click;
use crate::spaces::youtube::comments::CommentSection;
use crate::spaces::youtube::convert::{self, error_text};
use crate::spaces::youtube::history::{self, Reporter};
use crate::spaces::youtube::rating::{self, Rating};
use crate::spaces::youtube::watch::load_stream;
use crate::spaces::youtube::{Nav, Route, prefetch, save};
use ui::assets::Icon;
use ui::i18n::t;
use ui::images::Images;
use ui::picture::image_fill;
use ui::player::{PlayerEvent, PlayerView, Prefs};
use ui::{runtime, theme};

/// Height under the Short for its title and channel.
const INFO_HEIGHT: f32 = 64.;
/// Space around the Short, and between the columns.
const GAP: f32 = 16.;
/// The Short's least height, in a very small window.
const MIN_HEIGHT: f32 = 320.;
/// Width of the comments panel.
const COMMENTS_WIDTH: f32 = 420.;
/// Diameter of the round buttons beside the Short.
const ACTION_SIZE: f32 = 48.;
/// Width of their column, labels included.
const ACTIONS_WIDTH: f32 = ACTION_SIZE + 24.;
/// Width of the previous and next arrows' column, at the page's right edge.
const ARROWS_WIDTH: f32 = 40.;
/// A new wheel gesture starts after this long without wheel events: one gesture (a trackpad
/// swipe and its inertia, a burst of notches) moves by one Short.
const WHEEL_GAP: Duration = Duration::from_millis(250);
/// Length of the slide from one Short to the next.
const SLIDE: Duration = theme::SLOW;
/// The next page of Shorts loads when this few are left after the current one.
const LOAD_AHEAD: usize = 3;

/// What a Short tells the page.
pub enum ShortEvent {
  /// The comments button: open or close the panel.
  ToggleComments,
}

/// One Short: its player, metadata, rating and comments.
pub struct ShortView {
  client: Client,
  video_id: String,
  nav: Nav,
  player: Entity<PlayerView>,
  page: Option<WatchPage>,
  rating: Rating,
  comments: Entity<CommentSection>,
  comments_scroll: ScrollHandle,
  /// Reports the playback to the account's history.
  history: Reporter,
  /// The picture's width and height, from the page ([`short_size`]).
  size: (Pixels, Pixels),
  _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ShortEvent> for ShortView {}

impl ShortView {
  fn new(
    client: Client,
    video_id: String,
    nav: Nav,
    covered: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let settings = Settings::get(cx);
    let prefs = Prefs {
      subtitle_language: Some(settings.language.clone()),
      // A Short loops: nothing comes "up next".
      autoplay_next: false,
      ..Player::prefs(settings)
    };
    let player = cx.new(|cx| {
      let mut player = PlayerView::new(prefs, false, cx);
      player.offer_theater(false, cx);
      player.offer_speed(false, cx);
      player.set_covered(covered);
      player
    });
    let focus = player.read(cx).focus_handle().clone();
    window.focus(&focus, cx);
    let subscriptions = vec![
      cx.subscribe(&player, |this, player, event: &PlayerEvent, cx| {
        this.history.on_event(event, &player, cx);
        match *event {
          PlayerEvent::Ended => player.update(cx, |player, cx| player.replay(cx)),
          PlayerEvent::Sound { volume, muted } => Player::save_sound(volume, muted, cx),
          _ => {}
        }
      }),
    ];
    // The Short goes (the next one, another page, the tab closed): the last watch-time ping.
    cx.on_release(|this: &mut Self, cx| {
      let player = this.player.clone();
      this.history.close(&player, cx);
    })
    .detach();
    let comments_scroll = ScrollHandle::new();
    let comments = cx.new(|cx| {
      CommentSection::new(
        client.clone(),
        video_id.clone(),
        nav.clone(),
        player.clone(),
        comments_scroll.clone(),
        false,
        cx,
      )
    });
    // The comments button shows their count once the first page came.
    cx.observe(&comments, |_, _, cx| cx.notify()).detach();
    let history = Reporter::new(client.clone(), video_id.clone());
    let view = Self {
      client,
      video_id,
      nav,
      player,
      page: None,
      rating: Rating::default(),
      comments,
      comments_scroll,
      history,
      size: (px(0.), px(0.)),
      _subscriptions: subscriptions,
    };
    // A Short starts at its beginning, as on YouTube.
    load_stream(&view.client, &view.video_id, None, &view.player, cx);
    history::report(
      &view.history,
      |this| (&mut this.history, this.player.clone()),
      cx,
    );
    view.load_page(cx);
    view
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
              this
                .player
                .update(cx, |player, cx| player.set_title(title, cx));
              this.rating = Rating::of(&page);
              let token = page.comments_continuation.clone();
              this.comments.update(cx, |c, cx| c.start(token, cx));
              this.page = Some(page);
            }
            Some(Err(e)) => tracing::warn!("short page: {e}"),
            None => {}
          }
          cx.notify();
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

  /// The column of round buttons right of the Short.
  fn actions(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let status = self.rating.status;
    let likes = self.page.as_ref().and_then(|page| {
      if status == LikeStatus::Like {
        page.likes_liked.clone()
      } else {
        page.likes.clone()
      }
    });
    let comments = self
      .comments
      .read(cx)
      .count()
      .map(count_only)
      .unwrap_or_else(|| t("Comments", "Commentaires").to_owned());
    let (like_tip, dislike_tip) = match status {
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
    let video = Route::Watch(self.video_id.clone());
    v_flex()
      .flex_none()
      .gap_4()
      .items_center()
      .child(
        action(
          "short-like",
          Icon::ThumbsUp,
          like_tip,
          status == LikeStatus::Like,
        )
        .on_click(cx.listener(|this, _, window, cx| this.rate(LikeStatus::Like, window, cx)))
        .child(action_label(
          likes.unwrap_or_else(|| t("Like", "J'aime").to_owned()),
        )),
      )
      .child(
        action(
          "short-dislike",
          Icon::ThumbsDown,
          dislike_tip,
          status == LikeStatus::Dislike,
        )
        .on_click(cx.listener(|this, _, window, cx| this.rate(LikeStatus::Dislike, window, cx)))
        .child(action_label(t("Dislike", "Je n'aime pas"))),
      )
      .child(
        action(
          "short-comments",
          Icon::Comments,
          t("Comments", "Commentaires"),
          false,
        )
        .on_click(cx.listener(|_, _, _, cx| cx.emit(ShortEvent::ToggleComments)))
        .child(action_label(comments)),
      )
      .child(
        div()
          .size(px(ACTION_SIZE))
          .flex()
          .items_center()
          .justify_center()
          .rounded_full()
          .bg(theme::surface())
          .child(save::watch_menu_button(&self.video_id)),
      )
      .child({
        let nav = self.nav.clone();
        action(
          "short-as-video",
          Icon::Theater,
          t("Watch as a video", "Regarder en vidéo"),
          false,
        )
        .on_aux_click(middle_click({
          let (nav, video) = (nav.clone(), video.clone());
          move |cx| nav.open_in_new_tab(video.clone(), None, cx)
        }))
        .on_click(move |_, window, cx| nav.open(video.clone(), window, cx))
      })
  }

  /// The title and the channel under the Short.
  fn info(&self, width: Pixels, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    let Some(page) = &self.page else {
      return v_flex()
        .w(width)
        .h(px(INFO_HEIGHT))
        .pt_3()
        .gap_2()
        .child(Skeleton::new().w(px(160.)).h(px(16.)))
        .child(Skeleton::new().w(width * 0.7).h(px(14.)))
        .into_any_element();
    };
    let avatar = (!page.channel_avatar.is_empty()).then(|| {
      div().size(px(24.)).flex_none().child(image_fill(
        &convert::picture(&page.channel_avatar),
        24.,
        1.,
        px(12.),
        window,
        cx,
      ))
    });
    let channel = page.channel_id.clone().map(|id| Route::Channel {
      id,
      name: page.channel.clone().unwrap_or_default(),
    });
    v_flex()
      .w(width)
      .h(px(INFO_HEIGHT))
      .pt_3()
      .gap_1()
      .child(
        h_flex()
          .id("short-channel")
          .group("short-channel")
          .gap_2()
          .items_center()
          .text_sm()
          .font_weight(FontWeight::SEMIBOLD)
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
            div()
              .min_w_0()
              .truncate()
              .group_hover("short-channel", |s| s.text_color(theme::accent_hover()))
              .child(page.channel.clone().unwrap_or_default()),
          ),
      )
      .child(div().text_sm().line_clamp(1).child(page.title.clone()))
      .into_any_element()
  }

  /// The comments panel, `height` tall.
  fn comments_panel(&self, height: Pixels, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let header = h_flex()
      .flex_none()
      .px_4()
      .py_2()
      .items_center()
      .justify_between()
      .border_b_1()
      .border_color(theme::border())
      .child(
        div()
          .font_weight(FontWeight::SEMIBOLD)
          .child(t("Comments", "Commentaires")),
      )
      .child(
        Button::new("short-comments-close")
          .ghost()
          .icon(IconName::Close)
          .tooltip(t("Close", "Fermer"))
          .on_click(cx.listener(|_, _, _, cx| cx.emit(ShortEvent::ToggleComments))),
      );
    let body = div()
      .id("short-comments")
      .size_full()
      .px_4()
      .pb_4()
      .overflow_y_scroll()
      .track_scroll(&self.comments_scroll)
      .on_scroll(cx.listener(|this, _, _, cx| {
        this
          .comments
          .update(cx, |c, cx| c.load_more_if_near_end(cx));
        cx.notify();
      }))
      .child(self.comments.clone());
    v_flex()
      .w(px(COMMENTS_WIDTH))
      .h(height)
      .flex_none()
      .rounded(theme::RADIUS_LG)
      .overflow_hidden()
      .bg(theme::surface())
      .border_1()
      .border_color(theme::border())
      .child(header)
      .child(
        div()
          .flex_1()
          .min_h_0()
          .relative()
          .child(body)
          .vertical_scrollbar(&self.comments_scroll),
      )
  }
}

/// A round button beside the Short; its label goes under it as a child.
fn action(
  id: &'static str,
  icon: Icon,
  tooltip: &'static str,
  active: bool,
) -> gpui::Stateful<gpui::Div> {
  v_flex()
    .id(id)
    .group(id)
    .gap_1()
    .items_center()
    .cursor_pointer()
    .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
    .child(
      div()
        .size(px(ACTION_SIZE))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(theme::surface())
        .group_hover(id, |s| s.bg(theme::surface_hover()))
        .when(active, |el| el.text_color(theme::accent()))
        .child(gpui_component::Icon::new(icon).size(px(22.))),
    )
}

/// The text under a round button.
fn action_label(text: impl Into<SharedString>) -> impl IntoElement {
  div()
    .max_w(px(ACTIONS_WIDTH))
    .truncate()
    .text_xs()
    .font_weight(FontWeight::MEDIUM)
    .child(text.into())
}

/// The number of YouTube's localized count, without its noun: `2,3 M commentaires` → `2,3 M`.
fn count_only(count: &str) -> String {
  let count = count.trim();
  match count.rsplit_once(char::is_whitespace) {
    Some((number, noun)) if !noun.chars().any(|c| c.is_ascii_digit()) => {
      number.trim_end().to_owned()
    }
    _ => count.to_owned(),
  }
}

impl Render for ShortView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let (width, height) = self.size;
    if window.is_fullscreen() {
      return div()
        .size_full()
        .child(self.player.clone())
        .into_any_element();
    }
    h_flex()
      .flex_none()
      .gap(px(GAP))
      .items_end()
      .child(
        v_flex()
          .child(
            div()
              .w(width)
              .h(height)
              .rounded(theme::RADIUS_LG)
              .overflow_hidden()
              .bg(gpui::black())
              .child(self.player.clone()),
          )
          .child(self.info(width, window, cx)),
      )
      // Level with the bottom of the picture, as on YouTube.
      .child(div().pb(px(INFO_HEIGHT)).child(self.actions(cx)))
      .into_any_element()
  }
}

/// The Short's picture, 9:16 on the page's height, narrower when the width left by the sidebar,
/// the buttons, the comments panel (`comments_open`) and the arrows (kept free on both sides, so
/// that the Short stays centered) is short.
fn short_size(comments_open: bool, window: &Window, cx: &App) -> (Pixels, Pixels) {
  let viewport = window.viewport_size();
  let tall = viewport.height - theme::TITLE_BAR_HEIGHT - px(INFO_HEIGHT + 2. * GAP);
  let panel = if comments_open {
    COMMENTS_WIDTH + GAP
  } else {
    0.
  };
  let wide = viewport.width
    - theme::sidebar_width(Settings::get(cx).sidebar_collapsed)
    - px(2. * (GAP + ARROWS_WIDTH + GAP) + ACTIONS_WIDTH + GAP + panel);
  let height = tall.min(wide * (16. / 9.)).max(px(MIN_HEIGHT));
  (height * (9. / 16.), height)
}

/// The page: the sequence of Shorts and the one on screen.
pub struct ShortsView {
  client: Client,
  nav: Nav,
  ids: Vec<String>,
  continuation: Option<String>,
  /// A page of the sequence is being fetched.
  loading: bool,
  /// The first page failed: why, and a button retries.
  error: Option<SharedString>,
  /// Index in `ids` of the Short on screen.
  index: usize,
  current: Option<Entity<ShortView>>,
  comments_open: bool,
  /// Another page is on top: the Short stays paused, as the next one would.
  covered: bool,
  /// The last wheel event, for [`WHEEL_GAP`].
  last_wheel: Option<Instant>,
  /// The slide running, if any.
  slide: Option<Slide>,
  /// Counts the slides: each one gets its own animation id.
  slides: usize,
  _subscription: Option<Subscription>,
}

/// A slide from one Short to the next ([`SLIDE`]).
struct Slide {
  /// The Short leaving, paused; dropped when the slide ends.
  from: Entity<ShortView>,
  /// Toward the next Short (the Shorts move up), else the previous one (down).
  forward: bool,
  /// Its number in [`ShortsView::slides`].
  id: usize,
}

impl ShortsView {
  pub fn new(client: Client, nav: Nav, cx: &mut Context<Self>) -> Self {
    let mut view = Self {
      client,
      nav,
      ids: Vec::new(),
      continuation: None,
      loading: false,
      error: None,
      index: 0,
      current: None,
      comments_open: false,
      covered: false,
      last_wheel: None,
      slide: None,
      slides: 0,
      _subscription: None,
    };
    view.load(cx);
    view
  }

  /// The player of the Short on screen.
  pub fn player(&self, cx: &gpui::App) -> Option<Entity<PlayerView>> {
    Some(self.current.as_ref()?.read(cx).player.clone())
  }

  /// Pauses the Short while another page covers this one.
  pub fn set_covered(&mut self, covered: bool, cx: &mut Context<Self>) {
    self.covered = covered;
    if let Some(player) = self.player(cx) {
      player.update(cx, |player, _| player.set_covered(covered));
    }
  }

  /// The next page of the sequence: its start, then the Shorts after the last one.
  fn load(&mut self, cx: &mut Context<Self>) {
    if self.loading || (!self.ids.is_empty() && self.continuation.is_none()) {
      return;
    }
    self.loading = true;
    let client = self.client.clone();
    let token = self.continuation.clone();
    let fetch = runtime::spawn(cx, async move {
      match token {
        Some(token) => client.shorts_more(&token).await,
        None => client.shorts().await,
      }
    });
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this.update(cx, |this, cx| this.loaded(result, cx)).ok();
    })
    .detach();
    cx.notify();
  }

  fn loaded(
    &mut self,
    result: Option<Result<ShortsPage, innertube::Error>>,
    cx: &mut Context<Self>,
  ) {
    self.loading = false;
    match result {
      Some(Ok(page)) => {
        tracing::info!("shorts: {} more", page.ids.len());
        let known = self.ids.len();
        // The sequence may repeat one: each Short plays once.
        for id in page.ids {
          if !self.ids.contains(&id) {
            self.ids.push(id);
          }
        }
        self.continuation = page.continuation;
        // The first Short shows when the page renders ([`Self::show`]).
        if self.current.is_some() {
          self.warm_next(cx);
        }
        // The start alone (one Short): its sequence at once, for the next ones. A page with
        // nothing new stops there, until the user moves on.
        if self.ids.len() > known && self.ids.len() <= self.index + LOAD_AHEAD {
          self.load(cx);
        }
      }
      Some(Err(e)) => {
        tracing::warn!("shorts: {e}");
        if self.ids.is_empty() {
          self.error = Some(error_text(&e).into());
        }
      }
      None => {}
    }
    cx.notify();
  }

  fn retry(&mut self, cx: &mut Context<Self>) {
    self.error = None;
    self.load(cx);
    cx.notify();
  }

  /// Shows the Short at `index`: a new view and player, the previous one dropped.
  fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let Some(id) = self.ids.get(self.index).cloned() else {
      return;
    };
    let (client, nav, covered) = (self.client.clone(), self.nav.clone(), self.covered);
    let short = cx.new(|cx| ShortView::new(client, id, nav, covered, window, cx));
    self._subscription = Some(cx.subscribe(
      &short,
      |this, _, event: &ShortEvent, cx| match event {
        ShortEvent::ToggleComments => {
          this.comments_open = !this.comments_open;
          cx.notify();
        }
      },
    ));
    self.current = Some(short);
    self.warm_next(cx);
    if self.ids.len() <= self.index + LOAD_AHEAD {
      self.load(cx);
    }
    cx.notify();
  }

  /// Resolves the next Short's stream, so it starts at once.
  fn warm_next(&self, cx: &mut Context<Self>) {
    if let Some(next) = self.ids.get(self.index + 1) {
      prefetch::warm(next, cx);
    }
  }

  /// Moves `by` Shorts (1: the next, -1: the previous).
  fn step(&mut self, by: isize, window: &mut Window, cx: &mut Context<Self>) {
    let Some(index) = self.index.checked_add_signed(by) else {
      return;
    };
    if index == self.index {
      return;
    }
    // Past the last one known: the next ones (again, after a failure or an empty page).
    if index >= self.ids.len() {
      return self.load(cx);
    }
    self.index = index;
    let from = self.current.take();
    self.show(window, cx);
    if let Some(from) = from.filter(|_| !window.is_fullscreen()) {
      self.start_slide(from, by > 0, cx);
    }
  }

  /// Slides `from` off the page and the new Short in; `from` goes when the slide ends.
  fn start_slide(&mut self, from: Entity<ShortView>, forward: bool, cx: &mut Context<Self>) {
    from.update(cx, |short, cx| {
      short
        .player
        .update(cx, |player, _| player.set_covered(true));
    });
    self.slides += 1;
    let id = self.slides;
    self.slide = Some(Slide { from, forward, id });
    cx.spawn(async move |this, cx| {
      cx.background_executor().timer(SLIDE).await;
      this
        .update(cx, |this, cx| {
          if this.slide.as_ref().is_some_and(|slide| slide.id == id) {
            this.slide = None;
            cx.notify();
          }
        })
        .ok();
    })
    .detach();
  }

  fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
    let by = match event.keystroke.key.as_str() {
      "up" => -1,
      "down" => 1,
      _ => return,
    };
    if window.is_fullscreen() {
      return;
    }
    cx.stop_propagation();
    self.step(by, window, cx);
  }

  fn on_wheel(&mut self, event: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
    let dy = f32::from(event.delta.pixel_delta(window.line_height()).y);
    if dy.abs() < 0.5 {
      return;
    }
    let now = Instant::now();
    let new_gesture = self
      .last_wheel
      .is_none_or(|last| now.duration_since(last) > WHEEL_GAP);
    self.last_wheel = Some(now);
    if new_gesture {
      // The wheel turned toward the user (content up): the next Short.
      self.step(if dy < 0. { 1 } else { -1 }, window, cx);
    }
  }

  /// The previous and next arrows at the page's right edge.
  fn arrows(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let has_previous = self.index > 0;
    // Past the last one known, the button asks for the next ones ([`Self::step`]).
    let has_next = self.index + 1 < self.ids.len() || self.continuation.is_some();
    v_flex()
      .flex_none()
      .gap_3()
      .child(
        Button::new("short-previous")
          .icon(IconName::ChevronUp)
          .rounded_full()
          .disabled(!has_previous)
          .tooltip(t("Previous Short (↑)", "Short précédent (↑)"))
          .on_click(cx.listener(|this, _, window, cx| this.step(-1, window, cx))),
      )
      .child(
        Button::new("short-next")
          .icon(IconName::ChevronDown)
          .rounded_full()
          .disabled(!has_next)
          .tooltip(t("Next Short (↓)", "Short suivant (↓)"))
          .on_click(cx.listener(|this, _, window, cx| this.step(1, window, cx))),
      )
  }
}

impl Render for ShortsView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    // The first Short, once known: built here, so that a tab opened in the background plays
    // nothing.
    if self.current.is_none() && !self.ids.is_empty() {
      self.show(window, cx);
    }
    let Some(short) = self.current.clone() else {
      let content = match &self.error {
        Some(error) => v_flex()
          .gap_3()
          .items_center()
          .child(div().text_color(theme::muted()).child(error.clone()))
          .child(
            Button::new("shorts-retry")
              .label(t("Retry", "Réessayer"))
              .on_click(cx.listener(|this, _, _, cx| this.retry(cx))),
          )
          .into_any_element(),
        None => {
          let (width, height) = short_size(self.comments_open, window, cx);
          Skeleton::new()
            .w(width)
            .h(height)
            .rounded(theme::RADIUS_LG)
            .into_any_element()
        }
      };
      return div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(content)
        .into_any_element();
    };
    if window.is_fullscreen() {
      return div()
        .id("shorts")
        .size_full()
        .capture_key_down(cx.listener(Self::on_key))
        .child(short)
        .into_any_element();
    }
    let size = short_size(self.comments_open, window, cx);
    let height = size.1;
    let comments_open = self.comments_open;
    let panel = short.update(cx, |short, cx| {
      short.size = size;
      comments_open.then(|| short.comments_panel(height, cx))
    });
    // The Short coming in, and the one leaving above or below it while the slide runs. Both
    // move by the page's height, so that they leave and enter from off the page.
    let distance = f32::from(window.viewport_size().height - theme::TITLE_BAR_HEIGHT);
    let shorts = match &self.slide {
      Some(slide) => {
        let sign = if slide.forward { 1. } else { -1. };
        // A page away from the new one: where it was when the slide starts, off the page when
        // it ends.
        let leaving = div()
          .absolute()
          .top(px(-sign * distance))
          .left_0()
          .child(slide.from.clone());
        div()
          .relative()
          .child(short)
          .child(leaving)
          .with_animation(
            SharedString::from(format!("short-slide-{}", slide.id)),
            Animation::new(SLIDE).with_easing(ease_out_quint()),
            move |el, t| el.top(px(sign * distance * (1. - t))),
          )
          .into_any_element()
      }
      None => short.into_any_element(),
    };
    h_flex()
      .id("shorts")
      .size_full()
      .relative()
      .overflow_hidden()
      .p(px(GAP))
      .gap(px(GAP))
      .items_start()
      .justify_center()
      .capture_key_down(cx.listener(Self::on_key))
      .child(
        div()
          .id("shorts-wheel")
          .on_scroll(cx.listener(Self::on_wheel))
          .child(shorts),
      )
      .children(panel)
      .child(
        div()
          .absolute()
          .top(px(GAP))
          .right(px(GAP))
          .h(height)
          .flex()
          .items_center()
          .child(self.arrows(cx)),
      )
      .into_any_element()
  }
}

#[cfg(test)]
mod tests {
  use super::count_only;

  #[test]
  fn counts_without_their_noun() {
    assert_eq!(count_only("2,3\u{a0}M commentaires"), "2,3\u{a0}M");
    assert_eq!(
      count_only("2\u{202f}461\u{202f}262\u{a0}commentaires"),
      "2\u{202f}461\u{202f}262"
    );
    assert_eq!(count_only("1,204 comments"), "1,204");
    assert_eq!(count_only("12"), "12");
  }
}
