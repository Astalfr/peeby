//! The user's rating of a video (like, dislike), as the watch and Shorts pages set it: the button
//! changes at once and goes back if YouTube refuses.

use gpui::{Context, Window};
use gpui_component::WindowExt;
use gpui_component::notification::Notification;
use innertube::{Client, LikeParams, LikeStatus, WatchPage};

use crate::convert::error_text;
use ui::i18n::t;
use ui::{runtime, tf};

/// The rating a page shows.
#[derive(Default)]
pub struct Rating {
  /// As shown: the page's, then each click's until YouTube says otherwise.
  pub status: LikeStatus,
  /// What the like and dislike buttons send (the watch page's).
  params: LikeParams,
  /// A rating is waiting for YouTube's answer: clicks send nothing meanwhile.
  pending: bool,
}

impl Rating {
  /// The rating of `page`, as YouTube gave it.
  pub fn of(page: &WatchPage) -> Self {
    Self {
      status: page.like_status,
      params: page.like_params.clone(),
      pending: false,
    }
  }
}

/// A click on the like or dislike button of `video_id`: sets that rating, or clears it when it
/// is the current one. `slot` gives the view's [`Rating`], [`Rating::of`] its watch page.
pub fn rate<V: 'static>(
  view: &mut V,
  slot: fn(&mut V) -> &mut Rating,
  client: &Client,
  video_id: &str,
  clicked: LikeStatus,
  window: &mut Window,
  cx: &mut Context<V>,
) {
  if !client.signed_in() {
    let note = Notification::info(t(
      "To rate videos, sign in to YouTube in the settings.",
      "Pour noter les vidéos, connectez-vous à YouTube dans les réglages.",
    ));
    return window.push_notification(note, cx);
  }
  let rating = slot(view);
  if rating.pending {
    return;
  }
  let previous = rating.status;
  let status = if previous == clicked {
    LikeStatus::Indifferent
  } else {
    clicked
  };
  let params = rating.params.get(status).map(str::to_owned);
  rating.status = status;
  rating.pending = true;
  cx.notify();
  let (client, id) = (client.clone(), video_id.to_owned());
  let fetch = runtime::spawn(cx, async move {
    client.rate(&id, status, params.as_deref()).await
  });
  let handle = window.window_handle();
  cx.spawn(async move |this, cx| {
    let result = fetch.await;
    this
      .update(cx, |this, cx| {
        let rating = slot(this);
        rating.pending = false;
        if let Some(Ok(())) = result {
          return cx.notify();
        }
        rating.status = previous;
        cx.notify();
        if let Some(Err(e)) = result {
          tracing::warn!("rate: {e}");
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
