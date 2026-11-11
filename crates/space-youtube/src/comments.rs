//! The comments of a video, under the watch page's details or in the Shorts page's panel: pages
//! load as the panel scrolls near its end, replies when a thread opens. The links of comments
//! and descriptions ([`Links`], [`linked_text`]) live here too: a timestamp of the video seeks
//! its player, YouTube pages open in the tab, other sites in the browser. Signed in, the thumbs
//! rate a comment: the button changes at once and goes back if YouTube refuses. "Translate"
//! shows a comment in the client's language, translated by Google, until "See original".
//!
//! Signed in, a field over the comments posts one ("Comment"), "Reply" under a comment opens a
//! field under it, and the user's own comments have "Delete" (a second click confirms): public
//! writes from the account, each sent once its button is clicked, the list following YouTube's
//! answer (the new comment on top, the reply in its thread, the deleted one gone).

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
  App, Context, Entity, FontWeight, HighlightStyle, InteractiveText, ScrollHandle, SharedString,
  StyledText, Subscription, UnderlineStyle, WeakEntity, Window, div, point, px,
};
use gpui_component::WindowExt;
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::notification::Notification;
use gpui_component::tooltip::Tooltip;
use gpui_component::{Disableable, h_flex, v_flex};
use innertube::{Client, Comment, CommentComposer, LikeStatus, LinkTarget, TextLink};

use crate::convert::{self, error_text};
use crate::rating::{DISLIKE_RED, LIKE_BLUE};
use crate::{Nav, Route};
use ui::assets::Icon;
use ui::i18n::{Lang, lang, t};
use ui::images::Images;
use ui::picture::image_fill;
use ui::player::PlayerView;
use ui::widgets::{loading_more, near_scroll_end, sort_choices};
use ui::{runtime, tf, theme};

#[derive(Default)]
struct Comments {
  items: Vec<Comment>,
  count: Option<String>,
  /// The orders the header offers, from the first page.
  sorts: Vec<innertube::CommentSort>,
  /// The header's field, signed in.
  composer: Option<CommentComposer>,
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

/// A comment's translation ([`Client::translate_comment`]), by comment id.
enum Translation {
  /// Asked for: the button says so and sends nothing more.
  Loading,
  /// Shown in place of the text.
  Shown(String),
  /// Back to the original; the translation is kept for the next click.
  Hidden(String),
}

/// A link of a text under the pointer: the id of its text and its index there.
pub type HoveredLink = Option<(SharedString, usize)>;

/// Tells a view which of its links is hovered.
type SetHovered = dyn Fn(HoveredLink, &mut App);

/// The comments of one video.
pub struct CommentSection {
  client: Client,
  video_id: String,
  nav: Nav,
  player: Entity<PlayerView>,
  /// The scroll of the panel holding the section: the next page loads near its end.
  scroll: ScrollHandle,
  /// A timestamp's seek scrolls the panel back to the top, where the player is (the watch
  /// page; not the Shorts page, whose player is beside the panel).
  top_on_seek: bool,
  /// Token of the first page ([`innertube::WatchPage::comments_continuation`]), once known.
  first: Option<String>,
  comments: Comments,
  /// By comment id.
  replies: HashMap<String, Replies>,
  /// Comments whose rating waits for YouTube's answer: their thumbs send nothing meanwhile.
  rating: HashSet<String>,
  /// By comment id.
  translations: HashMap<String, Translation>,
  hovered_link: HoveredLink,
  /// The field over the comments, made once the header offers one (signed in).
  composer_input: Option<Entity<InputState>>,
  /// A comment or a reply is being sent: the buttons wait.
  posting: bool,
  /// The reply field open under a comment.
  reply: Option<ReplyBox>,
  /// The comment whose "Delete" was clicked once: the next click deletes it.
  confirming_delete: Option<String>,
  /// Comments being deleted.
  deleting: HashSet<String>,
  _composer_subscription: Option<Subscription>,
}

/// The reply field under a comment.
struct ReplyBox {
  /// The thread the reply goes to (the comment's, or its parent's for a reply).
  thread: String,
  /// The comment replied to, whose params it sends and under which it shows.
  target: String,
  params: String,
  input: Entity<InputState>,
  _subscription: Subscription,
}

impl CommentSection {
  /// The comments of `video_id` played by `player`, in a panel scrolled by `scroll`, back to its
  /// top on a timestamp's seek when `top_on_seek`. Nothing loads before
  /// [`CommentSection::start`].
  pub fn new(
    client: Client,
    video_id: String,
    nav: Nav,
    player: Entity<PlayerView>,
    scroll: ScrollHandle,
    top_on_seek: bool,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    Self {
      client,
      video_id,
      nav,
      player,
      scroll,
      top_on_seek,
      first: None,
      comments: Comments::default(),
      replies: HashMap::new(),
      rating: HashSet::new(),
      translations: HashMap::new(),
      hovered_link: None,
      composer_input: None,
      posting: false,
      reply: None,
      confirming_delete: None,
      deleting: HashSet::new(),
      _composer_subscription: None,
    }
  }

  /// The field over the comments, once the header offers one: Enter posts too.
  fn ensure_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.composer_input.is_some() {
      return;
    }
    let Some(composer) = &self.comments.composer else {
      return;
    };
    let placeholder = composer
      .placeholder
      .clone()
      .unwrap_or_else(|| t("Add a comment…", "Ajoutez un commentaire…").into());
    let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
    self._composer_subscription = Some(cx.subscribe_in(
      &input,
      window,
      |this, _, event: &InputEvent, window, cx| match event {
        InputEvent::PressEnter { .. } => this.post_comment(window, cx),
        InputEvent::Change => cx.notify(),
        _ => {}
      },
    ));
    self.composer_input = Some(input);
  }

  /// Posts the field's text as a comment; on YouTube's answer, the comment shows on top and the
  /// field empties; refused, a toast says so and the text stays.
  fn post_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let (Some(input), Some(composer)) = (self.composer_input.clone(), &self.comments.composer)
    else {
      return;
    };
    let text = input.read(cx).value().trim().to_owned();
    if text.is_empty() || self.posting {
      return;
    }
    self.posting = true;
    cx.notify();
    let (client, params) = (self.client.clone(), composer.params.clone());
    let fetch = runtime::spawn(
      cx,
      async move { client.create_comment(&params, &text).await },
    );
    let handle = window.window_handle();
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      let posted = this
        .update(cx, |this, cx| {
          this.posting = false;
          cx.notify();
          match result {
            Some(Ok(comment)) => {
              this.comments.items.insert(0, comment);
              true
            }
            Some(Err(e)) => {
              tracing::warn!("post comment: {e}");
              false
            }
            None => false,
          }
        })
        .unwrap_or(false);
      handle
        .update(cx, |_, window, cx| {
          if posted {
            input.update(cx, |input, cx| input.set_value("", window, cx));
          } else {
            window.push_notification(
              Notification::error(t(
                "Cannot post your comment.",
                "Impossible de publier votre commentaire.",
              )),
              cx,
            );
          }
        })
        .ok();
    })
    .detach();
  }

  /// Opens the reply field under comment `target` of `thread`, focused.
  fn open_reply(
    &mut self,
    thread: String,
    target: String,
    params: String,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if self.reply.as_ref().is_some_and(|r| r.target == target) {
      return;
    }
    let input = cx
      .new(|cx| InputState::new(window, cx).placeholder(t("Add a reply…", "Ajoutez une réponse…")));
    input.update(cx, |input, cx| input.focus(window, cx));
    let subscription = cx.subscribe_in(
      &input,
      window,
      |this, _, event: &InputEvent, window, cx| match event {
        InputEvent::PressEnter { .. } => this.post_reply(window, cx),
        InputEvent::Change => cx.notify(),
        _ => {}
      },
    );
    self.reply = Some(ReplyBox {
      thread,
      target,
      params,
      input,
      _subscription: subscription,
    });
    cx.notify();
  }

  fn close_reply(&mut self, cx: &mut Context<Self>) {
    self.reply = None;
    cx.notify();
  }

  /// Posts the reply field's text; on YouTube's answer, the reply shows in its thread (opened,
  /// its other replies loading if they never did) and the field closes.
  fn post_reply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let Some(reply) = &self.reply else { return };
    let text = reply.input.read(cx).value().trim().to_owned();
    if text.is_empty() || self.posting {
      return;
    }
    self.posting = true;
    cx.notify();
    let (client, params, thread) = (
      self.client.clone(),
      reply.params.clone(),
      reply.thread.clone(),
    );
    let fetch = runtime::spawn(cx, async move {
      client.create_comment_reply(&params, &text).await
    });
    let handle = window.window_handle();
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      let posted = this
        .update(cx, |this, cx| {
          this.posting = false;
          cx.notify();
          let reply = match result {
            Some(Ok(reply)) => reply,
            Some(Err(e)) => {
              tracing::warn!("post reply: {e}");
              return false;
            }
            None => return false,
          };
          this.reply = None;
          let continuation = this
            .comments
            .items
            .iter()
            .find(|c| c.id == thread)
            .and_then(|c| c.replies_continuation.clone());
          let replies = this.replies.entry(thread.clone()).or_default();
          // Never opened: its other replies load too, after the new one.
          let load = replies.items.is_empty() && !replies.loading && continuation.is_some();
          if load {
            replies.continuation = continuation;
          }
          replies.open = true;
          replies.items.insert(0, reply);
          if load {
            this.load_replies(thread.clone(), cx);
          }
          true
        })
        .unwrap_or(false);
      if !posted {
        handle
          .update(cx, |_, window, cx| {
            window.push_notification(
              Notification::error(t(
                "Cannot post your reply.",
                "Impossible de publier votre réponse.",
              )),
              cx,
            );
          })
          .ok();
      }
    })
    .detach();
  }

  /// "Delete" under one of the user's comments: the first click asks for a second, which
  /// deletes it; it goes from the list once YouTube removed it.
  fn delete(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
    if self.deleting.contains(id) {
      return;
    }
    if self.confirming_delete.as_deref() != Some(id) {
      self.confirming_delete = Some(id.to_owned());
      return cx.notify();
    }
    self.confirming_delete = None;
    let Some(action) = self.comment_mut(id).and_then(|c| c.delete_action.clone()) else {
      return;
    };
    self.deleting.insert(id.to_owned());
    cx.notify();
    let client = self.client.clone();
    let fetch = runtime::spawn(cx, async move { client.delete_comment(&action).await });
    let (handle, id) = (window.window_handle(), id.to_owned());
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      let deleted = this
        .update(cx, |this, cx| {
          this.deleting.remove(&id);
          cx.notify();
          match result {
            Some(Ok(())) => {
              this.comments.items.retain(|c| c.id != id);
              for replies in this.replies.values_mut() {
                replies.items.retain(|c| c.id != id);
              }
              true
            }
            Some(Err(e)) => {
              tracing::warn!("delete comment: {e}");
              false
            }
            None => false,
          }
        })
        .unwrap_or(false);
      let note = if deleted {
        Notification::info(t("Comment deleted", "Commentaire supprimé"))
      } else {
        Notification::error(t(
          "Cannot delete the comment.",
          "Impossible de supprimer le commentaire.",
        ))
      };
      handle
        .update(cx, |_, window, cx| window.push_notification(note, cx))
        .ok();
    })
    .detach();
  }

  /// A field and its buttons ("Cancel" while it holds text or is a reply's, then the send
  /// button, `label`), greyed while a post is under way.
  fn field(
    &self,
    key: &str,
    input: &Entity<InputState>,
    label: &'static str,
    reply: bool,
    cx: &mut Context<Self>,
  ) -> impl IntoElement + use<> {
    let empty = input.read(cx).value().trim().is_empty();
    let cancel = (reply || !empty).then(|| {
      let input = input.clone();
      Button::new(SharedString::from(format!("{key}-cancel")))
        .ghost()
        .label(t("Cancel", "Annuler"))
        .on_click(cx.listener(move |this, _, window, cx| {
          if reply {
            this.close_reply(cx);
          } else {
            input.update(cx, |input, cx| input.set_value("", window, cx));
          }
        }))
    });
    let send = Button::new(SharedString::from(format!("{key}-send")))
      .primary()
      .label(label)
      .disabled(empty || self.posting)
      .on_click(cx.listener(move |this, _, window, cx| {
        if reply {
          this.post_reply(window, cx);
        } else {
          this.post_comment(window, cx);
        }
      }));
    v_flex()
      .gap_2()
      .child(Input::new(input))
      .child(h_flex().gap_2().justify_end().children(cancel).child(send))
  }

  /// The reply field, when open under comment `id`.
  fn reply_field(&self, id: &str, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
    let reply = self.reply.as_ref().filter(|r| r.target == id)?;
    let input = reply.input.clone();
    Some(div().pl(px(48.)).child(self.field(
      &format!("reply-{id}"),
      &input,
      t("Reply", "Répondre"),
      true,
      cx,
    )))
  }

  /// Loads the first page from `token` (the watch page's); `None` when the video has none.
  pub fn start(&mut self, token: Option<String>, cx: &mut Context<Self>) {
    self.first = token;
    self.load_comments(cx);
    cx.notify();
  }

  /// The comments again from the first page of the order whose token is `token`: the list
  /// empties, the total and the orders stay until the new page brings its own.
  fn set_sort(&mut self, token: String, cx: &mut Context<Self>) {
    if self
      .comments
      .sorts
      .iter()
      .any(|s| s.selected && s.token == token)
    {
      return;
    }
    for sort in &mut self.comments.sorts {
      sort.selected = sort.token == token;
    }
    self.first = Some(token);
    let (count, sorts) = (
      self.comments.count.take(),
      std::mem::take(&mut self.comments.sorts),
    );
    let composer = self.comments.composer.take();
    self.comments = Comments {
      count,
      sorts,
      composer,
      ..Comments::default()
    };
    self.replies.clear();
    self.load_comments(cx);
    cx.notify();
  }

  /// YouTube's localized total, e.g. `2,3 M commentaires`, once the first page came.
  pub fn count(&self) -> Option<&str> {
    self.comments.count.as_deref()
  }

  /// Next comments page once the end of the page is within about a screen.
  pub fn load_more_if_near_end(&mut self, cx: &mut Context<Self>) {
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

  /// The comment `id`, a thread's or a reply.
  fn comment_mut(&mut self, id: &str) -> Option<&mut Comment> {
    self
      .comments
      .items
      .iter_mut()
      .chain(self.replies.values_mut().flat_map(|r| r.items.iter_mut()))
      .find(|c| c.id == id)
  }

  /// A click on a thumb of comment `id`: sets that rating, or clears it when it is the current
  /// one.
  fn rate(&mut self, id: &str, clicked: LikeStatus, window: &mut Window, cx: &mut Context<Self>) {
    if !self.client.signed_in() {
      let note = Notification::info(t(
        "To rate comments, sign in to YouTube in the settings.",
        "Pour noter les commentaires, connectez-vous à YouTube dans les réglages.",
      ));
      return window.push_notification(note, cx);
    }
    if self.rating.contains(id) {
      return;
    }
    let Some(comment) = self.comment_mut(id) else {
      return;
    };
    let previous = comment.rating;
    let status = if previous == clicked {
      LikeStatus::Indifferent
    } else {
      clicked
    };
    let Some(action) = comment
      .actions
      .as_ref()
      .and_then(|a| a.get(previous, status))
      .map(str::to_owned)
    else {
      return;
    };
    comment.rating = status;
    self.rating.insert(id.to_owned());
    cx.notify();
    let client = self.client.clone();
    let fetch = runtime::spawn(cx, async move { client.rate_comment(&action).await });
    let handle = window.window_handle();
    let id = id.to_owned();
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |this, cx| {
          this.rating.remove(&id);
          if let Some(Ok(())) = result {
            return cx.notify();
          }
          if let Some(comment) = this.comment_mut(&id) {
            comment.rating = previous;
          }
          cx.notify();
          if let Some(Err(e)) = result {
            tracing::warn!("rate comment: {e}");
            let note = Notification::error(tf!(
              "Cannot save your rating. {}",
              "Impossible d'enregistrer votre avis. {}",
              error_text(&e)
            ));
            handle
              .update(cx, |_, window, cx| window.push_notification(note, cx))
              .ok();
          }
        })
        .ok();
    })
    .detach();
  }

  /// A click on "Translate" or "See original" under comment `id`: shows its translation, asked
  /// for the first time, or the original again.
  fn translate(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
    match self.translations.remove(id) {
      Some(Translation::Loading) => {
        self
          .translations
          .insert(id.to_owned(), Translation::Loading);
        return;
      }
      Some(Translation::Shown(text)) => {
        self
          .translations
          .insert(id.to_owned(), Translation::Hidden(text));
        return cx.notify();
      }
      Some(Translation::Hidden(text)) => {
        self
          .translations
          .insert(id.to_owned(), Translation::Shown(text));
        return cx.notify();
      }
      None => {}
    }
    let Some(text) = self.comment_mut(id).map(|c| c.text.clone()) else {
      return;
    };
    self
      .translations
      .insert(id.to_owned(), Translation::Loading);
    cx.notify();
    let client = self.client.clone();
    let (video_id, comment_id) = (self.video_id.clone(), id.to_owned());
    let fetch = runtime::spawn(cx, async move {
      client
        .translate_comment(&video_id, &comment_id, &text)
        .await
    });
    let handle = window.window_handle();
    let id = id.to_owned();
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |this, cx| {
          this.translations.remove(&id);
          cx.notify();
          match result {
            Some(Ok(text)) => {
              this.translations.insert(id, Translation::Shown(text));
            }
            Some(Err(e)) => {
              tracing::warn!("translate comment: {e}");
              let note = Notification::error(tf!(
                "Cannot translate this comment. {}",
                "Impossible de traduire ce commentaire. {}",
                error_text(&e)
              ));
              handle
                .update(cx, |_, window, cx| window.push_notification(note, cx))
                .ok();
            }
            None => {}
          }
        })
        .ok();
    })
    .detach();
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
              // The user's own reply, shown already, may come again.
              let known: HashSet<String> = replies.items.iter().map(|c| c.id.clone()).collect();
              replies
                .items
                .extend(page.comments.into_iter().filter(|c| !known.contains(&c.id)));
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
      self.first.clone()
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
              if !page.sorts.is_empty() {
                comments.sorts = page.sorts;
              }
              if page.composer.is_some() {
                comments.composer = page.composer;
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

  fn render_comments(
    &self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement + use<> {
    let comments = &self.comments;
    let view = cx.weak_entity();
    let sorts = (comments.sorts.len() > 1).then(|| {
      sort_choices(
        "comment-sort",
        comments
          .sorts
          .iter()
          .map(|sort| {
            let (token, view) = (sort.token.clone(), view.clone());
            let on_click: ui::form::OnClick = Box::new(move |_, cx| {
              view
                .update(cx, |this, cx| this.set_sort(token.clone(), cx))
                .ok();
            });
            (
              SharedString::from(sort.label.clone()),
              sort.selected,
              on_click,
            )
          })
          .collect::<Vec<_>>(),
      )
    });
    let header = h_flex()
      .gap_2()
      .items_center()
      .child(gpui_component::Icon::new(Icon::Comments).size(px(18.)))
      .child(
        div().text_lg().font_weight(FontWeight::SEMIBOLD).child(
          comments
            .count
            .clone()
            .unwrap_or_else(|| t("Comments", "Commentaires").into()),
        ),
      )
      .child(div().flex_1())
      .children(sorts);
    let composer = self
      .composer_input
      .clone()
      .map(|input| self.field("comment", &input, t("Comment", "Commenter"), false, cx));
    let items: Vec<_> = comments
      .items
      .iter()
      .map(|c| self.comment_thread(c, window, cx))
      .collect();
    // Pages load as the page scrolls; the button only shows to retry after a failure.
    let more = (comments.failed && comments.continuation.is_some()).then(|| {
      Button::new("more-comments")
        .ghost()
        .label(t("Retry", "Réessayer"))
        .on_click(cx.listener(|this, _, _, cx| this.load_comments(cx)))
    });
    let empty = (comments.items.is_empty() && !comments.loading && comments.requested).then(|| {
      div()
        .text_color(theme::muted())
        .child(t("No comments", "Aucun commentaire"))
    });
    v_flex()
      .pt_2()
      .gap_5()
      .child(header)
      .children(composer)
      .children(items)
      .children(empty)
      .when(comments.loading, |el| el.child(loading_more()))
      .children(more.map(|b| h_flex().child(b)))
  }

  /// A comment, its "N replies" toggle and, once opened, its replies.
  fn comment_thread(
    &self,
    comment: &Comment,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement + use<> {
    let links = self.links(cx);
    let section = cx.weak_entity();
    let replies = self.replies.get(&comment.id);
    let open = replies.is_some_and(|r| r.open);
    let toggle = comment.replies_continuation.as_ref().map(|_| {
      let label = if open {
        t("Hide replies", "Masquer les réponses").to_owned()
      } else {
        replies_label(&comment.replies)
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
        .map(|reply| {
          let translation = self.translations.get(&reply.id);
          let row = comment_row(
            reply,
            Some(&comment.id),
            self.row_state(&reply.id),
            translation,
            &links,
            &section,
            window,
            cx,
          );
          v_flex()
            .gap_2()
            .child(row)
            .children(self.reply_field(&reply.id, cx))
        })
        .collect();
      let more = r.continuation.as_ref().map(|_| {
        let id = comment.id.clone();
        Button::new(SharedString::from(format!("more-replies-{}", comment.id)))
          .ghost()
          .label(t("More replies", "Plus de réponses"))
          .on_click(cx.listener(move |this, _, _, cx| this.load_replies(id.clone(), cx)))
      });
      v_flex()
        .pl(px(48.))
        .gap_3()
        .children(rows)
        .when(r.loading, |el| el.child(loading_more()))
        .children(more.filter(|_| !r.loading).map(|b| h_flex().child(b)))
    });
    let field = self.reply_field(&comment.id, cx);
    v_flex()
      .gap_2()
      .child(comment_row(
        comment,
        None,
        self.row_state(&comment.id),
        self.translations.get(&comment.id),
        &links,
        &section,
        window,
        cx,
      ))
      .children(field)
      .children(toggle.map(|b| h_flex().pl(px(40.)).child(b)))
      .children(shown)
  }
  /// Where comment `id`'s delete stands.
  fn row_state(&self, id: &str) -> RowState {
    if self.deleting.contains(id) {
      RowState::Deleting
    } else if self.confirming_delete.as_deref() == Some(id) {
      RowState::Confirming
    } else {
      RowState::Idle
    }
  }

  /// What the section's links need to be drawn and followed.
  fn links(&self, cx: &Context<Self>) -> Links {
    Links::new(
      self.video_id.clone(),
      self.nav.clone(),
      self.player.clone(),
      self.top_on_seek.then(|| self.scroll.clone()),
      cx.weak_entity(),
      self.hovered_link.clone(),
      |this| &mut this.hovered_link,
    )
  }
}

impl Render for CommentSection {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    // Offsets from the last layout: enough to start the next page early.
    self.load_more_if_near_end(cx);
    self.ensure_composer(window, cx);
    self.render_comments(window, cx)
  }
}

/// Follows the links of the description and the comments. Clicks never borrow the page:
/// opening another page updates it (it gets covered).
#[derive(Clone)]
pub struct Links {
  video_id: String,
  nav: Nav,
  player: Entity<PlayerView>,
  /// Scrolled back to its top when a timestamp seeks, if any.
  scroll: Option<ScrollHandle>,
  /// Tells the view drawing the links which one is hovered.
  set_hovered: Rc<SetHovered>,
  /// The view's hovered link when it rendered.
  hovered: HoveredLink,
}

impl Links {
  /// The links of a view showing `video_id` in `player`, `scroll` back to its top when a
  /// timestamp seeks; `view` keeps the
  /// hovered one (`hovered` when it rendered) in the field `slot` gives.
  pub fn new<V: 'static>(
    video_id: String,
    nav: Nav,
    player: Entity<PlayerView>,
    scroll: Option<ScrollHandle>,
    view: WeakEntity<V>,
    hovered: HoveredLink,
    slot: fn(&mut V) -> &mut HoveredLink,
  ) -> Self {
    let set_hovered = Rc::new(move |link: HoveredLink, cx: &mut App| {
      view
        .update(cx, |this, cx| {
          let hovered = slot(this);
          if *hovered != link {
            *hovered = link;
            cx.notify();
          }
        })
        .ok();
    });
    Self {
      video_id,
      nav,
      player,
      scroll,
      set_hovered,
      hovered,
    }
  }

  /// Underlines link `index` of the text `id` (none: `None`).
  fn hover(&self, link: HoveredLink, cx: &mut App) {
    (self.set_hovered)(link, cx);
  }

  /// A timestamp of this video seeks (and scrolls the player into view), YouTube pages open in
  /// the tab, other sites in the browser.
  fn follow(&self, target: LinkTarget, window: &mut Window, cx: &mut App) {
    match target {
      LinkTarget::Video { id, start } if id == self.video_id => {
        let seconds = f64::from(start.unwrap_or(0));
        self
          .player
          .update(cx, |player, cx| player.seek_to(seconds, cx));
        if let Some(scroll) = &self.scroll {
          scroll.set_offset(point(px(0.), px(0.)));
        }
      }
      LinkTarget::Video { id, .. } => self.nav.open(Route::Watch(id), window, cx),
      LinkTarget::Channel { id, name } => self.nav.open(Route::Channel { id, name }, window, cx),
      LinkTarget::Playlist { id, title } => {
        self.nav.open(Route::Playlist { id, title }, window, cx)
      }
      LinkTarget::Url(url) | LinkTarget::Page(url) => cx.open_url(&url),
    }
  }
}

/// A comment: its author's avatar, name and age, its text (or its `translation`), its thumbs
/// and its translate button.
/// Where a comment's delete stands.
#[derive(Clone, Copy, PartialEq)]
enum RowState {
  Idle,
  /// "Delete" clicked once: the next click deletes.
  Confirming,
  Deleting,
}

#[allow(clippy::too_many_arguments)] // A comment's row: each argument is one aspect of it.
fn comment_row(
  comment: &Comment,
  thread: Option<&str>,
  state: RowState,
  translation: Option<&Translation>,
  links: &Links,
  section: &WeakEntity<CommentSection>,
  window: &mut Window,
  cx: &mut App,
) -> impl IntoElement + use<> {
  let avatar = comment.avatar.as_ref().map(|url| {
    let sizes = [innertube::Thumbnail {
      url: url.clone(),
      width: 88,
      height: 88,
    }];
    image_fill(&convert::picture(&sizes), 36., 1., px(18.), window, cx)
  });
  // As on YouTube: the like thumb with the count next to it (alone at no likes), then the
  // dislike thumb, never counted. A liked comment shows the count with the user's like.
  let likes = if comment.rating == LikeStatus::Like {
    &comment.likes_liked
  } else {
    &comment.likes
  };
  let thumbs = h_flex()
    .gap_1()
    .items_center()
    .text_xs()
    .text_color(theme::muted())
    .child(thumb(comment, section, LikeStatus::Like, likes))
    .child(thumb(comment, section, LikeStatus::Dislike, ""))
    .when(!comment.text.trim().is_empty(), |el| {
      el.child(translate_button(comment, translation, section))
    })
    .children(comment.reply_params.clone().map(|params| {
      let (section, id) = (section.clone(), comment.id.clone());
      // A reply to a reply goes to its thread.
      let thread = thread.unwrap_or(&comment.id).to_owned();
      text_action(
        format!("reply-{}", comment.id),
        t("Reply", "Répondre"),
        true,
        move |window, cx| {
          section
            .update(cx, |this, cx| {
              this.open_reply(thread.clone(), id.clone(), params.clone(), window, cx)
            })
            .ok();
        },
      )
    }))
    .children(comment.delete_action.as_ref().map(|_| {
      let (section, id) = (section.clone(), comment.id.clone());
      let label = match state {
        RowState::Idle => t("Delete", "Supprimer"),
        RowState::Confirming => t("Delete for good?", "Supprimer pour de bon ?"),
        RowState::Deleting => t("Deleting…", "Suppression…"),
      };
      text_action(
        format!("delete-{}", comment.id),
        label,
        state != RowState::Deleting,
        move |window, cx| {
          section
            .update(cx, |this, cx| this.delete(&id, window, cx))
            .ok();
        },
      )
    }));
  // The translation has no links: YouTube's answer carries none.
  let text = match translation {
    Some(Translation::Shown(text)) => linked_text(
      format!("comment-translated-{}", comment.id),
      text,
      &[],
      links,
    ),
    _ => linked_text(
      format!("comment-{}", comment.id),
      &comment.text,
      &comment.links,
      links,
    ),
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
                  el.px_2().rounded_full().bg(theme::surface_hover())
                })
                .child(comment.author.clone()),
            )
            .child(
              div()
                .text_color(theme::muted())
                .child(comment.published.clone()),
            ),
        )
        .child(div().child(text))
        .child(thumbs),
    )
}

/// The like or dislike thumb of `comment`, `count` beside it: in YouTube's blue or red when it
/// is the user's rating and on hover. Signed out, YouTube gives no actions: a click says to
/// sign in.
fn thumb(
  comment: &Comment,
  section: &WeakEntity<CommentSection>,
  status: LikeStatus,
  count: &str,
) -> impl IntoElement + use<> {
  let current = comment.rating == status;
  let (kind, icon, color, tooltip) = if status == LikeStatus::Dislike {
    let tooltip = if current {
      t("Remove dislike", "Retirer « Je n'aime pas »")
    } else {
      t("Dislike", "Je n'aime pas")
    };
    ("dislike", Icon::ThumbsDown, DISLIKE_RED, tooltip)
  } else {
    let tooltip = if current {
      t("Remove like", "Je n'aime plus")
    } else {
      t("Like", "J'aime")
    };
    ("like", Icon::ThumbsUp, LIKE_BLUE, tooltip)
  };
  let color = gpui::Hsla::from(gpui::rgb(color));
  let id = comment.id.clone();
  let section = section.clone();
  h_flex()
    .id(SharedString::from(format!("{kind}-{}", comment.id)))
    .h(px(28.))
    .px_2()
    .gap_1p5()
    .items_center()
    .rounded_full()
    .cursor_pointer()
    .hover(move |s| s.bg(color.opacity(0.14)).text_color(color))
    .when(current, |el| el.text_color(color))
    .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
    .on_click(move |_, window, cx| {
      section
        .update(cx, |this, cx| this.rate(&id, status, window, cx))
        .ok();
    })
    .child(gpui_component::Icon::new(icon).size(px(14.)))
    .when(!count.is_empty(), |el| el.child(count.to_owned()))
}

/// "Translate" under `comment`, "Translating…" while it waits, "See original" once translated.
/// A text button of a comment's row (Reply, Delete), as "Translate" is.
fn text_action(
  id: String,
  label: &'static str,
  enabled: bool,
  on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
  h_flex()
    .id(SharedString::from(id))
    .h(px(28.))
    .px_2()
    .items_center()
    .rounded_full()
    .when(enabled, |el| {
      el.cursor_pointer()
        .hover(|s| s.bg(theme::surface_hover()).text_color(theme::foreground()))
        .on_click(move |_, window, cx| on_click(window, cx))
    })
    .child(label)
}

fn translate_button(
  comment: &Comment,
  translation: Option<&Translation>,
  section: &WeakEntity<CommentSection>,
) -> impl IntoElement + use<> {
  let label = match translation {
    None | Some(Translation::Hidden(_)) => t("Translate", "Traduire"),
    Some(Translation::Loading) => t("Translating…", "Traduction…"),
    Some(Translation::Shown(_)) => t(
      "See original (translated by Google)",
      "Voir l'original (traduit par Google)",
    ),
  };
  let loading = matches!(translation, Some(Translation::Loading));
  let id = comment.id.clone();
  let section = section.clone();
  h_flex()
    .id(SharedString::from(format!("translate-{}", comment.id)))
    .h(px(28.))
    .px_2()
    .items_center()
    .rounded_full()
    .when(!loading, |el| {
      el.cursor_pointer()
        .hover(|s| s.bg(theme::surface_hover()).text_color(theme::foreground()))
    })
    .on_click(move |_, window, cx| {
      section
        .update(cx, |this, cx| this.translate(&id, window, cx))
        .ok();
    })
    .child(label)
}

/// `text` with its `links` in the accent color, the hovered one underlined, followed on a click
/// ([`Links::follow`]). `id` is unique on the page: the element keeps its mouse state by it.
pub fn linked_text(
  id: impl Into<SharedString>,
  text: &str,
  links: &[TextLink],
  follow: &Links,
) -> gpui::Stateful<gpui::Div> {
  let id: SharedString = id.into();
  // A `\r` (of `\r\n`) would draw as a glyph: a space keeps the ranges' byte offsets.
  let text = SharedString::from(text.replace('\r', " "));
  let hovered = follow
    .hovered
    .as_ref()
    .filter(|(text_id, _)| *text_id == id)
    .map(|(_, index)| *index);
  let styled = StyledText::new(text).with_highlights(links.iter().enumerate().map(|(i, l)| {
    let style = HighlightStyle {
      color: Some(theme::accent_hover()),
      underline: (hovered == Some(i)).then(|| UnderlineStyle {
        thickness: px(1.),
        color: Some(theme::accent_hover()),
        wavy: false,
      }),
      ..Default::default()
    };
    (l.range.clone(), style)
  }));
  let ranges: Vec<_> = links.iter().map(|l| l.range.clone()).collect();
  let targets: Vec<LinkTarget> = links.iter().map(|l| l.target.clone()).collect();
  let text = InteractiveText::new(id.clone(), styled)
    .on_hover({
      let follow = follow.clone();
      let ranges = ranges.clone();
      let id = id.clone();
      move |index, _, _, cx| {
        let link = index.and_then(|ix| ranges.iter().position(|r| r.contains(&ix)));
        follow.hover(link.map(|i| (id.clone(), i)), cx);
      }
    })
    .on_click(ranges, {
      let follow = follow.clone();
      move |index, window, cx| {
        if let Some(target) = targets.get(index).cloned() {
          follow.follow(target, window, cx);
        }
      }
    });
  // The text reports moves over it only: leaving it clears its underline.
  div()
    .id(SharedString::from(format!("{id}-links")))
    .on_hover({
      let follow = follow.clone();
      move |hovered, _, cx| {
        if !hovered
          && follow
            .hovered
            .as_ref()
            .is_some_and(|(text_id, _)| *text_id == id)
        {
          follow.hover(None, cx);
        }
      }
    })
    .child(text)
}

/// The toggle of a comment's replies: "3 replies", from YouTube's count (`1,2 k` stays
/// plural); singular for one (and for zero in French).
fn replies_label(count: &str) -> String {
  match (lang(), count.trim().parse::<u64>().ok()) {
    (Lang::En, Some(1)) => "1 reply".to_owned(),
    (Lang::Fr, Some(0 | 1)) => format!("{count} réponse"),
    _ => tf!("{count} replies", "{count} réponses"),
  }
}
