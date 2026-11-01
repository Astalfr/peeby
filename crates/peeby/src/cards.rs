//! YouTube videos as the shared media cards: a click opens the watch page, resting on a card
//! resolves its stream (so a click plays at once), and the save buttons sit on the thumbnail.

use gpui::{AnyElement, App, WeakEntity, Window};
use innertube::VideoItem;
use ui::card::{Hover, MediaCard, RowAction};

use crate::app::{AppView, Route};
use crate::{save, youtube};

/// Opens the watch page of `video`.
fn open_video(
  video: &VideoItem,
  nav: &WeakEntity<AppView>,
) -> impl Fn(&mut Window, &mut App) + use<> {
  let nav = nav.clone();
  let route = Route::Watch(video.id.clone());
  move |window, cx| {
    nav
      .update(cx, |app, cx| app.open(route.clone(), window, cx))
      .ok();
  }
}

/// The card of `video` under `id` (which tells apart the same video in two lists): title,
/// thumbnail, duration, channel then views · date, and the stream prefetch on hover.
fn card(id: &str, hover: Hover, video: &VideoItem) -> MediaCard {
  let video_id = video.id.clone();
  MediaCard::new(
    id.to_owned(),
    hover,
    video.title.clone(),
    youtube::picture(&video.thumbnails),
  )
  .lines([
    video.channel.clone().unwrap_or_default(),
    video.metadata.join(" · "),
  ])
  .badge(video.duration.clone())
  .on_hover(move |hovered, cx| crate::prefetch::hover(&video_id, hovered, cx))
}

/// Grid card, `width` wide, with the channel avatar and the save buttons.
pub fn video_card(
  video: &VideoItem,
  width: f32,
  nav: &WeakEntity<AppView>,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let id = format!("card-{}", video.id);
  let hover = Hover::new(&id, window, cx);
  let t = hover.t;
  card(&id, hover, video)
    .avatar(Some(youtube::picture(&video.avatar)))
    .overlay(save::watch_later_button(&id, &video.id, t, cx))
    .trailing(save::menu_button(&id, &video.id, t))
    .on_click(open_video(video, nav))
    .grid(width, window, cx)
}

/// Compact card (related videos).
pub fn compact_card(
  video: &VideoItem,
  nav: &WeakEntity<AppView>,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let id = format!("related-{}", video.id);
  let hover = Hover::new(&id, window, cx);
  card(&id, hover, video)
    .on_click(open_video(video, nav))
    .compact(window, cx)
}

/// List row (subscriptions in list mode, as YouTube showed them until about 2023), with the
/// channel avatar and the save buttons.
pub fn video_row(
  video: &VideoItem,
  nav: &WeakEntity<AppView>,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let id = format!("row-{}", video.id);
  let hover = Hover::new(&id, window, cx);
  let t = hover.t;
  card(&id, hover, video)
    .avatar(Some(youtube::picture(&video.avatar)))
    .overlay(save::watch_later_button(&id, &video.id, t, cx))
    .trailing(save::menu_button(&id, &video.id, t))
    .on_click(open_video(video, nav))
    .row(window, cx)
}

/// A video of a playlist (its page, or the queue next to the player). `id` must tell apart two
/// entries of the same video, which playlists allow. `current` marks the video playing.
#[allow(clippy::too_many_arguments)] // A row of a list: each argument is one aspect of it.
pub fn playlist_row(
  id: &str,
  number: usize,
  video: &VideoItem,
  thumb_width: f32,
  current: bool,
  on_click: impl Fn(&mut Window, &mut App) + 'static,
  on_remove: Option<RowAction>,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let hover = Hover::new(id, window, cx);
  card(id, hover, video).on_click(on_click).numbered(
    number,
    thumb_width,
    current,
    on_remove,
    window,
    cx,
  )
}
