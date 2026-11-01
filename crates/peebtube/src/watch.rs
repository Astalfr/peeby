//! Watch page: the player (autoplay), title, channel, like/dislike bar (Return YouTube
//! Dislike), description, comments and related videos. In fullscreen only the player is shown.
//!
//! The stream (VISIONOS HLS, ~0.2 s), the page metadata (`next`) and the votes load in
//! parallel; mpv starts as soon as the stream is known, so playback begins about 1 s after the
//! click.

use std::collections::HashMap;
use std::time::Instant;

use gpui::prelude::*;
use gpui::{
  App, Context, Entity, FontWeight, Global, ScrollHandle, SharedString, WeakEntity, Window, div,
  px, relative,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, v_flex};
use innertube::{Client, Comment, Votes, WatchPage};

use crate::app::{AppView, Route};
use crate::assets::Icon;
use crate::images::Images;
use crate::player_view::PlayerView;
use crate::ui::{compact_card, error_text, image_fill, near_scroll_end};
use crate::{runtime, theme, tools};

/// Highest video quality requested.
const MAX_HEIGHT: u32 = 1080;
/// Width of the related videos column.
const RELATED_WIDTH: f32 = 400.;

/// Return YouTube Dislike results for the session: its API allows 100 requests a minute and
/// 10,000 a day per client.
#[derive(Default)]
struct VotesCache(HashMap<String, Votes>);

impl Global for VotesCache {}

#[derive(Default)]
struct Comments {
  items: Vec<Comment>,
  count: Option<String>,
  continuation: Option<String>,
  loading: bool,
  /// Whether the first page was asked for.
  requested: bool,
  /// The last request failed: no automatic retry, the button retries.
  failed: bool,
}

/// Replies of one comment, loaded when first opened.
#[derive(Default)]
struct Replies {
  items: Vec<Comment>,
  continuation: Option<String>,
  loading: bool,
  open: bool,
}

pub struct WatchView {
  client: Client,
  video_id: String,
  nav: WeakEntity<AppView>,
  player: Entity<PlayerView>,
  page: Option<WatchPage>,
  votes: Option<Votes>,
  comments: Comments,
  /// By comment id.
  replies: HashMap<String, Replies>,
  scroll: ScrollHandle,
}

impl WatchView {
  pub fn new(
    client: Client,
    video_id: String,
    nav: WeakEntity<AppView>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let player = cx.new(PlayerView::new);
    let focus = player.read(cx).focus_handle().clone();
    window.focus(&focus, cx);
    let votes = cx
      .try_global::<VotesCache>()
      .and_then(|cache| cache.0.get(&video_id).copied());
    let view = Self {
      client,
      video_id,
      nav,
      player,
      page: None,
      votes,
      comments: Comments::default(),
      replies: HashMap::new(),
      scroll: ScrollHandle::new(),
    };
    view.load_stream(cx);
    view.load_page(cx);
    if view.votes.is_none() {
      view.load_votes(cx);
    }
    view
  }

  /// Pauses the player while another page covers this one.
  pub fn set_covered(&mut self, covered: bool, cx: &mut Context<Self>) {
    self
      .player
      .update(cx, |player, _| player.set_covered(covered));
  }

  fn load_stream(&self, cx: &mut Context<Self>) {
    let clicked = Instant::now();
    let (client, id) = (self.client.clone(), self.video_id.clone());
    let fetch = runtime::spawn(cx, async move { client.stream(&id, MAX_HEIGHT).await });
    let player = self.player.downgrade();
    let video_id = self.video_id.clone();
    cx.spawn(async move |_, cx| {
      let error = match fetch.await {
        Some(Ok(stream)) => {
          tracing::info!(
            "stream after {:.2} s: {} {}",
            clicked.elapsed().as_secs_f64(),
            stream.resolution,
            stream.codecs
          );
          player
            .update(cx, |player, cx| player.start(stream, clicked, cx))
            .ok();
          return;
        }
        Some(Err(e)) => e,
        None => return,
      };
      // Network down: yt-dlp would fail too. Anything else (age or membership restriction,
      // client blocked, unexpected response): let the bundled yt-dlp try (ADR 0005).
      let fallback = if matches!(error, innertube::Error::Http(_)) {
        None
      } else {
        tracing::info!("VISIONOS failed ({error}), trying yt-dlp");
        cx.background_executor()
          .spawn(async move { tools::ytdl_options(&video_id) })
          .await
      };
      player
        .update(cx, |player, cx| match fallback {
          Some(options) => player.start_with(options, clicked, cx),
          None => {
            tracing::warn!("stream: {error}");
            player.fail(error_text(&error).into(), cx);
          }
        })
        .ok();
    })
    .detach();
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
              this.page = Some(page);
              this.load_comments(cx);
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

  /// Next comments page once the end of the page is within about a screen.
  fn load_more_if_near_end(&mut self, cx: &mut Context<Self>) {
    let c = &self.comments;
    if c.requested
      && !c.loading
      && !c.failed
      && c.continuation.is_some()
      && near_scroll_end(&self.scroll)
    {
      self.load_comments(cx);
    }
  }

  /// Opens or closes the replies of `comment`, loading them the first time.
  fn toggle_replies(&mut self, comment: &Comment, cx: &mut Context<Self>) {
    let replies = self.replies.entry(comment.id.clone()).or_default();
    replies.open = !replies.open;
    if replies.open && replies.items.is_empty() && !replies.loading {
      replies.continuation = comment.replies_continuation.clone();
      self.load_replies(comment.id.clone(), cx);
    }
    cx.notify();
  }

  /// Next page of replies of comment `id`.
  fn load_replies(&mut self, id: String, cx: &mut Context<Self>) {
    let Some(replies) = self.replies.get_mut(&id) else {
      return;
    };
    let Some(token) = replies.continuation.take() else {
      return;
    };
    replies.loading = true;
    let client = self.client.clone();
    let fetch = runtime::spawn(cx, async move { client.comments(&token).await });
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |this, cx| {
          let Some(replies) = this.replies.get_mut(&id) else {
            return;
          };
          replies.loading = false;
          match result {
            Some(Ok(page)) => {
              replies.items.extend(page.comments);
              replies.continuation = page.continuation;
            }
            Some(Err(e)) => tracing::warn!("replies: {e}"),
            None => {}
          }
          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  /// First page, then the next ones as the page scrolls.
  fn load_comments(&mut self, cx: &mut Context<Self>) {
    let token = if self.comments.requested {
      self.comments.continuation.clone()
    } else {
      self
        .page
        .as_ref()
        .and_then(|p| p.comments_continuation.clone())
    };
    let Some(token) = token else { return };
    if self.comments.loading {
      return;
    }
    self.comments.requested = true;
    self.comments.loading = true;
    self.comments.failed = false;
    let client = self.client.clone();
    let fetch = runtime::spawn(cx, async move { client.comments(&token).await });
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |this, cx| {
          let comments = &mut this.comments;
          comments.loading = false;
          match result {
            Some(Ok(page)) => {
              tracing::info!("comments: {} more", page.comments.len());
              comments.items.extend(page.comments);
              comments.continuation = page.continuation;
              if page.count.is_some() {
                comments.count = page.count;
              }
            }
            Some(Err(e)) => {
              tracing::warn!("comments: {e}");
              comments.failed = true;
            }
            None => {}
          }
          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  fn details(&self, window: &mut Window, cx: &mut App) -> impl IntoElement + use<> {
    let Some(page) = &self.page else {
      return v_flex();
    };
    let avatar = (!page.channel_avatar.is_empty()).then(|| {
      div().size(px(40.)).flex_none().child(image_fill(
        &page.channel_avatar,
        40.,
        px(20.),
        window,
        cx,
      ))
    });
    let channel = page.channel_id.clone().map(|id| {
      let nav = self.nav.clone();
      let name = page.channel.clone().unwrap_or_default();
      move |_: &gpui::ClickEvent, window: &mut Window, cx: &mut App| {
        let route = Route::Channel {
          id: id.clone(),
          name: name.clone(),
        };
        nav.update(cx, |app, cx| app.open(route, window, cx)).ok();
      }
    });
    let stats = [page.views.clone(), page.date.clone()]
      .into_iter()
      .flatten()
      .collect::<Vec<_>>()
      .join(" · ");
    v_flex()
      .gap_3()
      .child(
        div()
          .text_xl()
          .font_weight(FontWeight::SEMIBOLD)
          .child(page.title.clone()),
      )
      .child(
        h_flex()
          .gap_3()
          .items_center()
          .child(
            h_flex()
              .id("channel")
              .gap_3()
              .items_center()
              .when_some(channel, |el, on_click| {
                el.cursor_pointer().on_click(on_click)
              })
              .children(avatar)
              .child(
                v_flex()
                  .child(
                    div()
                      .font_weight(FontWeight::SEMIBOLD)
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
          .child(div().flex_1())
          .children(self.votes.map(votes_bar)),
      )
      .child(
        v_flex()
          .p_3()
          .gap_1()
          .rounded(theme::RADIUS_LG)
          .bg(theme::surface())
          .text_sm()
          .child(div().font_weight(FontWeight::SEMIBOLD).child(stats))
          // `line_clamp` limits wrapped lines, not explicit line breaks: cut those first.
          .child(
            div().line_clamp(4).child(
              page
                .description
                .lines()
                .take(4)
                .collect::<Vec<_>>()
                .join("\n"),
            ),
          ),
      )
  }

  fn comments(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let comments = &self.comments;
    let header = h_flex()
      .gap_2()
      .items_center()
      .child(gpui_component::Icon::new(Icon::Comments).size(px(18.)))
      .child(
        div().text_lg().font_weight(FontWeight::SEMIBOLD).child(
          comments
            .count
            .clone()
            .unwrap_or_else(|| "Commentaires".into()),
        ),
      );
    let items: Vec<_> = comments
      .items
      .iter()
      .map(|c| self.comment_thread(c, window, cx))
      .collect();
    // Pages load as the page scrolls; the button only shows to retry after a failure.
    let more = (comments.failed && comments.continuation.is_some()).then(|| {
      Button::new("more-comments")
        .ghost()
        .label("Réessayer")
        .on_click(cx.listener(|this, _, _, cx| this.load_comments(cx)))
    });
    let empty = (comments.items.is_empty() && !comments.loading && comments.requested)
      .then(|| div().text_color(theme::muted()).child("Aucun commentaire"));
    v_flex()
      .gap_4()
      .child(header)
      .children(items)
      .children(empty)
      .when(comments.loading, |el| {
        el.child(div().text_color(theme::muted()).child("Chargement…"))
      })
      .children(more.map(|b| h_flex().child(b)))
  }

  /// A comment, its "N réponses" toggle and, once opened, its replies.
  fn comment_thread(
    &self,
    comment: &Comment,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement + use<> {
    let replies = self.replies.get(&comment.id);
    let open = replies.is_some_and(|r| r.open);
    let toggle = comment.replies_continuation.as_ref().map(|_| {
      let label = if open {
        "Masquer les réponses".to_owned()
      } else {
        format!("{} réponses", comment.replies)
      };
      let target = comment.clone();
      Button::new(SharedString::from(format!("replies-{}", comment.id)))
        .ghost()
        .label(label)
        .on_click(cx.listener(move |this, _, _, cx| this.toggle_replies(&target, cx)))
    });
    let shown = replies.filter(|r| r.open).map(|r| {
      let rows: Vec<_> = r
        .items
        .iter()
        .map(|reply| comment_row(reply, window, cx))
        .collect();
      let more = r.continuation.as_ref().map(|_| {
        let id = comment.id.clone();
        Button::new(SharedString::from(format!("more-replies-{}", comment.id)))
          .ghost()
          .label("Plus de réponses")
          .on_click(cx.listener(move |this, _, _, cx| this.load_replies(id.clone(), cx)))
      });
      v_flex()
        .pl(px(48.))
        .gap_3()
        .children(rows)
        .when(r.loading, |el| {
          el.child(
            div()
              .text_sm()
              .text_color(theme::muted())
              .child("Chargement…"),
          )
        })
        .children(more.filter(|_| !r.loading).map(|b| h_flex().child(b)))
    });
    v_flex()
      .gap_2()
      .child(comment_row(comment, window, cx))
      .children(toggle.map(|b| h_flex().pl(px(40.)).child(b)))
      .children(shown)
  }
}

/// Likes, estimated dislikes and the ratio bar, with the attribution RYD requires.
fn votes_bar(votes: Votes) -> impl IntoElement {
  let total = votes.likes + votes.dislikes;
  let ratio = if total > 0 {
    votes.likes as f32 / total as f32
  } else {
    1.0
  };
  v_flex()
    .id("votes")
    .w(px(220.))
    .gap_1()
    .cursor_pointer()
    .on_click(|_, _, cx| cx.open_url("https://returnyoutubedislike.com"))
    .child(
      h_flex()
        .justify_between()
        .child(
          h_flex()
            .gap_1()
            .items_center()
            .child(gpui_component::Icon::new(Icon::ThumbsUp).size(px(16.)))
            .child(compact_number(votes.likes)),
        )
        .child(
          h_flex()
            .gap_1()
            .items_center()
            .child(compact_number(votes.dislikes))
            .child(gpui_component::Icon::new(Icon::ThumbsDown).size(px(16.))),
        ),
    )
    .child(
      div()
        .h(px(4.))
        .w_full()
        .rounded_full()
        .bg(theme::border())
        .child(
          div()
            .h_full()
            .w(relative(ratio))
            .rounded_full()
            .bg(theme::foreground()),
        ),
    )
    .child(
      div()
        .text_xs()
        .text_color(theme::muted())
        .child("Dislikes : Return YouTube Dislike"),
    )
}

fn comment_row(comment: &Comment, window: &mut Window, cx: &mut App) -> impl IntoElement + use<> {
  let avatar = comment.avatar.as_ref().map(|url| {
    let sizes = [innertube::Thumbnail {
      url: url.clone(),
      width: 88,
      height: 88,
    }];
    image_fill(&sizes, 36., px(18.), window, cx)
  });
  let meta = if comment.likes.is_empty() {
    String::new()
  } else {
    format!("{} j'aime", comment.likes)
  };
  h_flex()
    .gap_3()
    .items_start()
    .child(div().size(px(36.)).flex_none().children(avatar))
    .child(
      v_flex()
        .min_w_0()
        .gap_1()
        .child(
          h_flex()
            .gap_2()
            .text_sm()
            .child(
              div()
                .font_weight(FontWeight::SEMIBOLD)
                .when(comment.by_creator, |el| {
                  el.px_1().rounded(px(4.)).bg(theme::surface_hover())
                })
                .child(comment.author.clone()),
            )
            .child(
              div()
                .text_color(theme::muted())
                .child(comment.published.clone()),
            ),
        )
        .child(div().child(comment.text.clone()))
        .when(!meta.is_empty(), |el| {
          el.child(div().text_sm().text_color(theme::muted()).child(meta))
        }),
    )
}

/// `1,2 k`, `3,4 M`: French short counts.
fn compact_number(n: u64) -> String {
  let short = |value: f64, unit: &str| {
    let text = if value < 10.0 {
      format!("{value:.1}")
    } else {
      format!("{value:.0}")
    };
    format!("{} {unit}", text.replace('.', ",").trim_end_matches(",0"))
  };
  match n {
    0..1_000 => n.to_string(),
    1_000..1_000_000 => short(n as f64 / 1e3, "k"),
    1_000_000..1_000_000_000 => short(n as f64 / 1e6, "M"),
    _ => short(n as f64 / 1e9, "Md"),
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
    let related: Vec<_> = self
      .page
      .iter()
      .flat_map(|page| page.related.iter())
      .map(|v| compact_card(v, &self.nav, window, cx))
      .collect();
    // Offsets from the last layout: enough to start the next comments page early.
    self.load_more_if_near_end(cx);
    let details = self.details(window, cx);
    let comments = self.comments(window, cx);
    let page = v_flex()
      .id("watch")
      .size_full()
      .overflow_y_scroll()
      .track_scroll(&self.scroll)
      .on_scroll(cx.listener(|this, _, _, cx| {
        this.load_more_if_near_end(cx);
        cx.notify();
      }))
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
              .gap_4()
              // 16:9 box: GPUI has no aspect-ratio style, so the height follows the width
              // through padding-top as a share of the width.
              .child(
                div()
                  .relative()
                  .w_full()
                  .pt(relative(9. / 16.))
                  .rounded(theme::RADIUS_LG)
                  .overflow_hidden()
                  .child(div().absolute().inset_0().child(self.player.clone())),
              )
              .child(details)
              .child(comments),
          )
          .child(
            v_flex()
              .w(px(RELATED_WIDTH))
              .flex_none()
              .gap_3()
              .children(related),
          ),
      );
    div()
      .size_full()
      .relative()
      .vertical_scrollbar(&self.scroll)
      .child(page)
      .into_any_element()
  }
}

#[cfg(test)]
mod tests {
  use super::compact_number;

  #[test]
  fn compact_numbers() {
    assert_eq!(compact_number(999), "999");
    assert_eq!(compact_number(1_234), "1,2 k");
    assert_eq!(compact_number(19_491_081), "19 M");
    assert_eq!(compact_number(521_536), "522 k");
    assert_eq!(compact_number(2_000_000), "2 M");
  }
}
